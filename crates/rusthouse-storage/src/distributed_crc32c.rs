//! Streaming CRC32C for accidental corruption checks, not authentication.
use std::io::{self, Read};

const POLYNOMIAL: u32 = 0x82f6_3b78;

// Build the Castagnoli lookup table at compile time. Replication verifies every
// copied byte, so avoid an eight-iteration bit loop for every input byte.
const TABLE: [u32; 256] = make_table();
const SLICING_TABLES: [[u32; 256]; 8] = make_slicing_tables();

const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < table.len() {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = (value >> 1) ^ if value & 1 == 1 { POLYNOMIAL } else { 0 };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

const fn make_slicing_tables() -> [[u32; 256]; 8] {
    let mut tables = [[0u32; 256]; 8];
    tables[0] = TABLE;
    let mut row = 1;
    while row < tables.len() {
        let mut index = 0;
        while index < 256 {
            let previous = tables[row - 1][index];
            tables[row][index] = (previous >> 8) ^ TABLE[(previous & 0xff) as usize];
            index += 1;
        }
        row += 1;
    }
    tables
}

struct Checksum(u32);

impl Checksum {
    fn new() -> Self {
        Self(!0)
    }

    fn append(&mut self, bytes: &[u8]) {
        self.0 = update(self.0, bytes);
    }

    fn finish(self) -> u32 {
        self.0 ^ !0
    }
}

/// Compute CRC32C (Castagnoli) for one byte slice.
pub fn crc32c(bytes: &[u8]) -> u32 {
    let mut checksum = Checksum::new();
    checksum.append(bytes);
    checksum.finish()
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "crc")]
unsafe fn update_arm64(mut crc: u32, bytes: &[u8]) -> u32 {
    use std::arch::aarch64::{__crc32cb, __crc32cd};

    let mut words = bytes.chunks_exact(8);
    for word in &mut words {
        let value = u64::from_le_bytes(word.try_into().expect("eight-byte chunk"));
        crc = __crc32cd(crc, value);
    }
    for &byte in words.remainder() {
        crc = __crc32cb(crc, byte);
    }
    crc
}

fn update(mut crc: u32, bytes: &[u8]) -> u32 {
    #[cfg(target_arch = "aarch64")]
    if bytes.len() >= 128 && std::arch::is_aarch64_feature_detected!("crc") {
        // SAFETY: CRC support was checked on the running CPU.
        return unsafe { update_arm64(crc, bytes) };
    }
    // A slicing-by-eight lookup processes two UInt32 words per iteration
    // without platform-specific instructions.
    let mut wide_chunks = bytes.chunks_exact(8);
    for chunk in &mut wide_chunks {
        let word = u64::from_le_bytes(chunk.try_into().expect("eight-byte chunk"));
        let value = u64::from(crc) ^ word;
        crc = SLICING_TABLES[7][(value & 0xff) as usize]
            ^ SLICING_TABLES[6][((value >> 8) & 0xff) as usize]
            ^ SLICING_TABLES[5][((value >> 16) & 0xff) as usize]
            ^ SLICING_TABLES[4][((value >> 24) & 0xff) as usize]
            ^ SLICING_TABLES[3][((value >> 32) & 0xff) as usize]
            ^ SLICING_TABLES[2][((value >> 40) & 0xff) as usize]
            ^ SLICING_TABLES[1][((value >> 48) & 0xff) as usize]
            ^ SLICING_TABLES[0][(value >> 56) as usize];
    }
    let mut four_chunks = wide_chunks.remainder().chunks_exact(4);
    for chunk in &mut four_chunks {
        let word = u32::from_le_bytes(chunk.try_into().expect("four-byte chunk"));
        let value = crc ^ word;
        crc = SLICING_TABLES[3][(value & 0xff) as usize]
            ^ SLICING_TABLES[2][((value >> 8) & 0xff) as usize]
            ^ SLICING_TABLES[1][((value >> 16) & 0xff) as usize]
            ^ SLICING_TABLES[0][(value >> 24) as usize];
    }
    for &byte in four_chunks.remainder() {
        let index = ((crc ^ u32::from(byte)) & 0xff) as usize;
        crc = (crc >> 8) ^ TABLE[index];
    }
    crc
}
/// Stream a file, reading at most one byte past max_bytes to detect oversized input.
/// Returns (length, checksum).
pub fn crc32c_reader(reader: &mut impl Read, max_bytes: u64) -> io::Result<(u64, u32)> {
    let mut checksum = Checksum::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let available = (max_bytes - total)
            .saturating_add(1)
            .min(buffer.len() as u64) as usize;
        let count = match reader.read(&mut buffer[..available]) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("transfer length overflow"))?;
        if total > max_bytes {
            return Err(io::Error::other("transfer exceeds configured byte cap"));
        }
        checksum.append(&buffer[..count]);
    }
    Ok((total, checksum.finish()))
}

/// Incremental CRC32C for an ordered byte stream.
/// The checksum is only a corruption check, not an authentication mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crc32cState {
    state: u32,
}

impl Crc32cState {
    pub fn new() -> Self {
        Self { state: !0u32 }
    }

    pub fn append(&mut self, bytes: &[u8]) {
        self.state = update(self.state, bytes);
    }

    /// Snapshot the current CRC32C without consuming the accumulator.
    pub fn checksum(&self) -> u32 {
        self.state ^ !0u32
    }
}

impl Default for Crc32cState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn arm64_crc_matches_across_unaligned_splits() {
        if !std::arch::is_aarch64_feature_detected!("crc") {
            return;
        }
        let bytes = (0..10_003).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        for split in [0, 1, 7, 8, 127, 128, 4096, bytes.len()] {
            // SAFETY: CRC support was checked on the running CPU.
            let first = unsafe { update_arm64(!0, &bytes[..split]) };
            let second = unsafe { update_arm64(first, &bytes[split..]) };
            assert_eq!(second ^ !0, crc32c(&bytes));
        }
    }

    #[test]
    fn matches_castagnoli_reference_vector() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
        assert_eq!(crc32c(b""), 0);
    }

    #[test]
    fn lookup_table_matches_bitwise_reference_across_chunk_boundaries() {
        fn bitwise(bytes: &[u8]) -> u32 {
            let mut crc = !0u32;
            for &byte in bytes {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ if crc & 1 == 1 { POLYNOMIAL } else { 0 };
                }
            }
            crc ^ !0
        }
        let bytes = (0..100_003).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        assert_eq!(crc32c(&bytes), bitwise(&bytes));
        for split in [0, 1, 8191, 8192, 32768, bytes.len()] {
            let mut crc = !0;
            crc = update(crc, &bytes[..split]);
            crc = update(crc, &bytes[split..]);
            assert_eq!(crc ^ !0, bitwise(&bytes));
        }
    }

    #[test]
    fn slicing_by_eight_matches_reference_at_every_short_length() {
        let bytes = (0..128).map(|index| (index * 37) as u8).collect::<Vec<_>>();
        let mut crc = !0u32;
        for (index, &byte) in bytes.iter().enumerate() {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ if crc & 1 == 1 { POLYNOMIAL } else { 0 };
            }
            assert_eq!(crc32c(&bytes[..=index]), crc ^ !0, "prefix {index}");
        }
    }

    #[test]
    fn oversized_input_reads_at_most_one_byte_beyond_cap() {
        struct Metered<'a> {
            source: Cursor<&'a [u8]>,
            consumed: usize,
        }

        impl Read for Metered<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let count = self.source.read(buf)?;
                self.consumed += count;
                Ok(count)
            }
        }

        let data = vec![7u8; 100_000];
        let mut reader = Metered {
            source: Cursor::new(&data),
            consumed: 0,
        };
        assert!(crc32c_reader(&mut reader, 5).is_err());
        assert_eq!(reader.consumed, 6);
    }

    #[test]
    fn large_replica_checksums_use_64k_reads() {
        struct Counting<'a> {
            input: &'a [u8],
            calls: usize,
        }
        impl Read for Counting<'_> {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                self.calls += 1;
                self.input.read(out)
            }
        }
        let bytes = vec![42; 128 * 1024];
        let mut source = Counting {
            input: &bytes,
            calls: 0,
        };
        assert_eq!(
            crc32c_reader(&mut source, bytes.len() as u64).unwrap(),
            (bytes.len() as u64, crc32c(&bytes))
        );
        assert_eq!(source.calls, 3); // two data reads and one EOF check
    }

    #[test]
    fn streaming_matches_in_memory_and_enforces_cap() {
        let data = vec![42u8; 75_000];
        assert_eq!(
            crc32c_reader(&mut Cursor::new(&data), data.len() as u64).unwrap(),
            (75_000, crc32c(&data))
        );
        assert!(crc32c_reader(&mut Cursor::new(data), 74_999).is_err());
    }
    #[test]
    fn interrupted_read_does_not_abort_streaming_checksum() {
        struct Once<'a> {
            cursor: Cursor<&'a [u8]>,
            interrupt: bool,
        }
        impl Read for Once<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.interrupt {
                    self.interrupt = false;
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.cursor.read(buf)
            }
        }
        let data = b"retry safe replica bytes";
        let mut reader = Once {
            cursor: Cursor::new(data.as_slice()),
            interrupt: true,
        };
        assert_eq!(
            crc32c_reader(&mut reader, data.len() as u64).unwrap(),
            (data.len() as u64, crc32c(data))
        );
    }
}

#[cfg(test)]
mod incremental_crc_tests {
    use super::*;

    #[test]
    fn matches_one_shot_crc_at_all_chunk_boundaries() {
        let data = b"123456789 plus some additional immutable storage bytes";
        for split in 0..=data.len() {
            let mut crc = Crc32cState::new();
            crc.append(&data[..split]);
            crc.append(&data[split..]);
            assert_eq!(crc.checksum(), crc32c(data), "split {split}");
        }
        assert_eq!(Crc32cState::default().checksum(), crc32c(b""));
    }

    #[test]
    fn can_continue_after_intermediate_checksum_snapshot() {
        let mut crc = Crc32cState::new();
        crc.append(b"part-one");
        assert_eq!(crc.checksum(), crc32c(b"part-one"));
        crc.append(b"-part-two");
        assert_eq!(crc.checksum(), crc32c(b"part-one-part-two"));
    }
}

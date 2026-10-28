//! Streaming CRC32C for accidental corruption checks, not authentication.
use std::io::{self, Read};

const POLYNOMIAL: u32 = 0x82f6_3b78;

// Build the Castagnoli lookup table at compile time. Replication verifies every
// copied byte, so avoid an eight-iteration bit loop for every input byte.
const TABLE: [u32; 256] = make_table();

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

fn update(mut crc: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
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
    let mut buffer = [0u8; 32 * 1024];
    loop {
        let available = (max_bytes - total)
            .saturating_add(1)
            .min(buffer.len() as u64) as usize;
        let count = reader.read(&mut buffer[..available])?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
    fn streaming_matches_in_memory_and_enforces_cap() {
        let data = vec![42u8; 75_000];
        assert_eq!(
            crc32c_reader(&mut Cursor::new(&data), data.len() as u64).unwrap(),
            (75_000, crc32c(&data))
        );
        assert!(crc32c_reader(&mut Cursor::new(data), 74_999).is_err());
    }
}
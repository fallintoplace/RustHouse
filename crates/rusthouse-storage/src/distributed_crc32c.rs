//! Streaming CRC32C for accidental corruption checks, not authentication.
use std::io::{self, Read};
use std::sync::OnceLock;

const POLYNOMIAL: u32 = 0x82f6_3b78;
const INITIAL_CRC32C: u32 = !0;

fn finalize_crc32c(state: u32) -> u32 {
    state ^ !0
}

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
        Self(INITIAL_CRC32C)
    }

    fn append(&mut self, bytes: &[u8]) {
        self.0 = update(self.0, bytes);
    }

    fn finish(self) -> u32 {
        finalize_crc32c(self.0)
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

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
unsafe fn update_sse42(mut crc: u32, bytes: &[u8]) -> u32 {
    use std::arch::x86_64::{_mm_crc32_u8, _mm_crc32_u64};

    let mut words = bytes.chunks_exact(8);
    for word in &mut words {
        let value = u64::from_le_bytes(word.try_into().expect("eight-byte chunk"));
        crc = _mm_crc32_u64(u64::from(crc), value) as u32;
    }
    for &byte in words.remainder() {
        crc = _mm_crc32_u8(crc, byte);
    }
    crc
}

fn update(crc: u32, bytes: &[u8]) -> u32 {
    #[cfg(target_arch = "aarch64")]
    if bytes.len() >= 128 && std::arch::is_aarch64_feature_detected!("crc") {
        // SAFETY: CRC support was checked on the running CPU.
        return unsafe { update_arm64(crc, bytes) };
    }
    #[cfg(target_arch = "x86_64")]
    if bytes.len() >= 128 && std::is_x86_feature_detected!("sse4.2") {
        // SAFETY: SSE4.2 support was checked on the running CPU.
        return unsafe { update_sse42(crc, bytes) };
    }
    update_software(crc, bytes)
}

fn update_software(mut crc: u32, bytes: &[u8]) -> u32 {
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

/// Apply a GF(2) operator to a 32-bit CRC state.
fn gf2_matrix_times(matrix: &[u32; 32], mut vector: u32) -> u32 {
    let mut result = 0u32;
    // Only the set bits contribute to a GF(2) dot product. Sparse CRC
    // states avoid testing every intervening zero bit.
    while vector != 0 {
        result ^= matrix[vector.trailing_zeros() as usize];
        vector &= vector - 1;
    }
    result
}

fn gf2_matrix_square(matrix: &[u32; 32]) -> [u32; 32] {
    std::array::from_fn(|index| gf2_matrix_times(matrix, matrix[index]))
}

/// Precomputed CRC32C zero-byte shift for repeatedly appending fixed-size chunks.
///
/// The ordinary crc32c_combine rebuilds GF(2) matrices for each length.
/// A transfer with thousands of equal-size verified chunks can reuse this
/// operator, reducing each combination to one 32-bit matrix-vector multiply.
#[derive(Clone)]
pub struct Crc32cShift {
    operator: [u32; 32],
}

impl Crc32cShift {
    /// Precompute the linear transformation for appending right_len bytes.
    pub fn new(mut right_len: u64) -> Self {
        let mut operator = std::array::from_fn(|bit| 1u32 << bit);
        if right_len == 0 {
            return Self { operator };
        }
        let mut one_bit = [0u32; 32];
        one_bit[0] = POLYNOMIAL;
        let mut row = 1u32;
        for slot in one_bit.iter_mut().skip(1) {
            *slot = row;
            row <<= 1;
        }
        // Squaring the one-bit operator three times gives eight zero bits.
        let mut power = gf2_matrix_square(&gf2_matrix_square(&gf2_matrix_square(&one_bit)));
        while right_len != 0 {
            if right_len & 1 != 0 {
                // Compose power after the existing operator.
                operator = std::array::from_fn(|bit| gf2_matrix_times(&power, operator[bit]));
            }
            right_len >>= 1;
            if right_len != 0 {
                power = gf2_matrix_square(&power);
            }
        }
        Self { operator }
    }

    /// Combine finalized checksums of adjacent ranges without re-reading bytes.
    /// The right-hand input must have the same length used in new().
    pub fn combine(&self, left: u32, right: u32) -> u32 {
        gf2_matrix_times(&self.operator, left) ^ right
    }
}

#[cfg(test)]
mod reusable_crc_shift_tests {
    use super::*;

    #[test]
    fn cached_shift_agrees_with_independent_crc_and_direct_combination() {
        let payload = (0..32768usize)
            .map(|i| ((i.wrapping_mul(77) + i / 7) % 251) as u8)
            .collect::<Vec<_>>();
        for right_len in [0usize, 1, 2, 3, 7, 8, 15, 64, 256, 4096, 8192] {
            let shift = Crc32cShift::new(right_len as u64);
            let right = &payload[..right_len];
            for left_len in [0usize, 1, 13, 31, 512, 4096, 16384] {
                let left = &payload[..left_len];
                let expected = crc32c(&[left, right].concat());
                assert_eq!(shift.combine(crc32c(left), crc32c(right)), expected,
                    "left={left_len} right={right_len}");
                assert_eq!(shift.combine(crc32c(left), crc32c(right)),
                    crc32c_combine(crc32c(left), crc32c(right), right_len as u64));
            }
        }
    }

    #[test]
    fn fixed_size_chunks_reduce_to_one_exact_stream_crc() {
        let payload = (0..128_000usize).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        for chunk_size in [1usize, 7, 64, 1000, 4096] {
            let shift = Crc32cShift::new(chunk_size as u64);
            let mut cumulative = crc32c(b"");
            for chunk in payload.chunks_exact(chunk_size) {
                cumulative = shift.combine(cumulative, crc32c(chunk));
            }
            let tail = payload.chunks_exact(chunk_size).remainder();
            cumulative = crc32c_combine(cumulative, crc32c(tail), tail.len() as u64);
            assert_eq!(cumulative, crc32c(&payload), "chunk_size={chunk_size}");
        }
    }

    #[test]
    fn byte_length_edges_match_generic_combine() {
        for length in [0u64, 1, 512, 65_536, 1 << 32, u64::MAX] {
            let shift = Crc32cShift::new(length);
            for crc in [0, 1, 0xdead_beef, u32::MAX] {
                assert_eq!(shift.combine(crc, 0),
                    crc32c_combine(crc, 0, length), "len={length}");
            }
        }
    }
}

/// Combine two finalized CRC32C values as if their byte ranges were concatenated.
///
/// The inputs must be checksums of consecutive byte ranges in the same order.
/// This permits parallel verification of chunks without re-reading the data.
/// A zero-length right range leaves the left checksum unchanged.
// The 64 powers of the one-zero-byte CRC shift cover every u64 byte length.
// Initialize once, rather than rebuilding GF(2) matrices for every merge.
static CRC32C_BYTE_SHIFTS: OnceLock<[[u32; 32]; 64]> = OnceLock::new();

fn crc32c_byte_shifts() -> &'static [[u32; 32]; 64] {
    CRC32C_BYTE_SHIFTS.get_or_init(|| {
        let mut powers = [[0u32; 32]; 64];
        powers[0] = Crc32cShift::new(1).operator;
        for bit in 1..powers.len() {
            powers[bit] = gf2_matrix_square(&powers[bit - 1]);
        }
        powers
    })
}

/// Combine two finalized CRC32C values as if their byte ranges were concatenated.
///
/// The inputs must be checksums of consecutive byte ranges in the same order.
/// This permits parallel verification of chunks without re-reading the data.
/// A zero-length right range leaves the left checksum unchanged.
pub fn crc32c_combine(mut left: u32, right: u32, mut right_len: u64) -> u32 {
    if right_len == 0 {
        return left;
    }
    let powers = crc32c_byte_shifts();
    let mut bit = 0;
    while right_len != 0 {
        if right_len & 1 != 0 {
            left = gf2_matrix_times(&powers[bit], left);
        }
        bit += 1;
        right_len >>= 1;
    }
    left ^ right
}

#[cfg(test)]
mod cached_crc_byte_shift_tests {
    use super::*;

    #[test]
    fn cached_powers_match_independent_shift_for_every_length_bit() {
        for bit in 0..64 {
            let len = 1u64 << bit;
            let expected = Crc32cShift::new(len);
            for left in [0, 1, 0x1234_5678, 0xffff_ffff] {
                for right in [0, 1, 0xabcd_ef01, 0xffff_ffff] {
                    assert_eq!(
                        crc32c_combine(left, right, len),
                        expected.combine(left, right),
                        "length bit {bit}"
                    );
                }
            }
        }
    }

    #[test]
    fn combined_bit_lengths_are_equivalent_and_order_independent() {
        for len in [0, 1, 3, 7, 9, 127, 255, 4095, 1 << 32, u64::MAX] {
            let shift = Crc32cShift::new(len);
            for left in [0, 1, 0xdead_beef, u32::MAX] {
                assert_eq!(
                    crc32c_combine(left, 0x1234_abcd, len),
                    shift.combine(left, 0x1234_abcd)
                );
            }
        }
    }
}

#[cfg(test)]
mod parallel_crc_combine_tests {
    use super::*;

    #[test]
    fn combining_chunks_matches_single_pass_for_every_split() {
        let data = (0..8193).map(|i| (i * 37 % 251) as u8).collect::<Vec<_>>();
        let expected = crc32c(&data);
        for split in 0..=data.len() {
            assert_eq!(
                crc32c_combine(
                    crc32c(&data[..split]),
                    crc32c(&data[split..]),
                    (data.len() - split) as u64,
                ),
                expected,
                "split {split}",
            );
        }
    }

    #[test]
    fn combines_independent_parallel_chunks_associatively() {
        let data = (0..131_075).map(|i| (i * 97 % 253) as u8).collect::<Vec<_>>();
        let partitions = [0, 1, 17, 4096, 8192, 131_074, 131_075];
        let mut reduced = crc32c(b"");
        let mut previous = 0;
        for &end in partitions.iter().skip(1) {
            reduced = crc32c_combine(
                reduced, crc32c(&data[previous..end]), (end - previous) as u64
            );
            previous = end;
        }
        assert_eq!(reduced, crc32c(&data));

        let left = crc32c(&data[..17]);
        let middle = crc32c(&data[17..4096]);
        let right = crc32c(&data[4096..]);
        let lhs = crc32c_combine(
            crc32c_combine(left, middle, 4096 - 17),
            right, (data.len() - 4096) as u64,
        );
        let rhs = crc32c_combine(
            left,
            crc32c_combine(middle, right, (data.len() - 4096) as u64),
            (data.len() - 17) as u64,
        );
        assert_eq!(lhs, crc32c(&data));
        assert_eq!(rhs, lhs);
    }

    #[test]
    fn zero_length_right_range_does_not_shift_checksum() {
        assert_eq!(crc32c_combine(0x1234_5678, 0, 0), 0x1234_5678);
        assert_eq!(crc32c_combine(0, crc32c(b"payload"), 7), crc32c(b"payload"));
    }
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
        Self { state: INITIAL_CRC32C }
    }

    pub fn append(&mut self, bytes: &[u8]) {
        self.state = update(self.state, bytes);
    }

    /// Snapshot the current CRC32C without consuming the accumulator.
    pub fn checksum(&self) -> u32 {
        finalize_crc32c(self.state)
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

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn sse42_crc_matches_software_for_all_streaming_boundaries() {
        if !std::is_x86_feature_detected!("sse4.2") {
            return;
        }
        let input = (0..16_389).map(|n| (n * 17 % 251) as u8).collect::<Vec<_>>();
        for split in [0, 1, 7, 8, 127, 128, 4096, input.len()] {
            // SAFETY: SSE4.2 is supported by the running CPU.
            let first = unsafe { update_sse42(!0, &input[..split]) };
            let second = unsafe { update_sse42(first, &input[split..]) };
            assert_eq!(second, update_software(!0, &input));
        }
    }

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

#[cfg(test)]
mod sparse_gf2_matrix_tests {
    use super::*;

    fn reference(matrix: &[u32; 32], mut vector: u32) -> u32 {
        let mut result = 0;
        for &value in matrix {
            if vector & 1 != 0 {
                result ^= value;
            }
            vector >>= 1;
        }
        result
    }

    #[test]
    fn set_bit_iteration_matches_full_32_bit_reference() {
        let matrix = std::array::from_fn(|index| {
            (index as u32).wrapping_mul(0x9e37_79b9).rotate_left(index as u32)
        });
        for vector in [0, 1, u32::MAX, 0x8000_0000, 0x8000_0001, 0xaaaa_aaaa, 0x5555_5555] {
            assert_eq!(gf2_matrix_times(&matrix, vector), reference(&matrix, vector));
        }
        for bit in 0..32 {
            assert_eq!(gf2_matrix_times(&matrix, 1u32 << bit), matrix[bit]);
        }
        for seed in 0u32..4096 {
            let vector = seed.wrapping_mul(0x85eb_ca6b).rotate_left(seed & 31);
            assert_eq!(gf2_matrix_times(&matrix, vector), reference(&matrix, vector));
        }
    }

    #[test]
    fn crc_combination_is_unchanged_for_sparse_and_dense_states() {
        for len in [0, 1, 7, 64, 8192, 1 << 20, u64::MAX] {
            let shift = Crc32cShift::new(len);
            for left in [0, 1, 0x8000_0000, 0x1234_5678, u32::MAX] {
                for right in [0, 0xdead_beef, u32::MAX] {
                    // The checksum of an empty right-hand range is zero.
                    if len == 0 && right != 0 {
                        continue;
                    }
                    assert_eq!(shift.combine(left, right), crc32c_combine(left, right, len));
                }
            }
        }
    }
}

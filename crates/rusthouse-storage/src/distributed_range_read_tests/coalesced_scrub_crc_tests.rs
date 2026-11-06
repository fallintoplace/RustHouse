use super::*;
use crate::distributed_crc32c::crc32c;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn overlapping_unordered_ranges_across_io_boundaries_match_independent_crc() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-coalesced-crc-{}-{}.bin",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let bytes = (0..(256 * 1024 + 37usize))
        .map(|index| (index * 29 % 251) as u8)
        .collect::<Vec<_>>();
    std::fs::write(&path, &bytes).unwrap();
    let spans = [
        (150_000u64, 100_000u64),
        (0, 240_000),
        (65_535, 131_072),
        (150_000, 0),
        (7, 200_000),
        (255_000, 800),
    ];
    let mut ranges = spans
        .iter()
        .map(|&(offset, length)| {
            let start = offset as usize;
            let expected = crc32c(&bytes[start..start + length as usize]);
            (offset, length, expected)
        })
        .collect::<Vec<_>>();
    verify_object_ranges_crc32c(&path, &ranges, 64 * 1024 * 1024).unwrap();
    ranges[2].2 ^= 1;
    assert_eq!(
        verify_object_ranges_crc32c(&path, &ranges, 64 * 1024 * 1024)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    std::fs::remove_file(&path).unwrap();
}

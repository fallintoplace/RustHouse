//! Safe seek-and-read support for a future replica object service.
use std::fs::{self, File};
use std::io;
#[cfg(not(unix))]
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const ABSOLUTE_MAX_RANGE: usize = 8 * 1024 * 1024;
const MAX_RANGES: usize = 1024;

fn validate_range(offset: u64, length: usize, file_length: u64) -> io::Result<()> {
    let end = offset
        .checked_add(length as u64)
        .ok_or_else(|| io::Error::other("object range overflow"))?;
    if end > file_length {
        return Err(io::Error::other("object range exceeds file length"));
    }
    Ok(())
}

#[cfg(unix)]
fn same_object_identity(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    before.dev() == after.dev() && before.ino() == after.ino()
}

fn object_state_unchanged(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    if !after.file_type().is_file()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return same_object_identity(before, after)
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec();
    }
    #[cfg(not(unix))]
    true
}

fn verify_object_after_read(path: &Path, file: &File, before: &fs::Metadata) -> io::Result<()> {
    let end_fd = file.metadata()?;
    let end_path = fs::symlink_metadata(path)?;
    if !object_state_unchanged(before, &end_fd)
        || !object_state_unchanged(before, &end_path)
    {
        return Err(io::Error::other("replica object changed during range read"));
    }
    Ok(())
}

fn open_verified_object(path: &Path) -> io::Result<(File, fs::Metadata)> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(io::Error::other("object is not a regular file"));
    }
    let file = File::open(path)?;
    let opened = file.metadata()?;
    // Checking only length and inode misses in-place overwrites between
    // the path stat and the open. Compare timestamps and change time too,
    // using the same verified identity contract as post-read checks.
    if !object_state_unchanged(&meta, &opened) {
        return Err(io::Error::other("object changed before range read"));
    }
    Ok((file, opened))
}

fn read_at(file: &mut File, offset: u64, length: usize) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; length];
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        // Positioned reads do not mutate the shared descriptor offset.
        // This removes seek syscalls for scattered immutable object ranges.
        file.read_exact_at(&mut bytes, offset)?;
    }
    #[cfg(not(unix))]
    {
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut bytes)?;
    }
    Ok(bytes)
}

/// Read an exact byte range from a frozen regular file, rejecting short reads.
pub fn read_object_range(
    path: &Path,
    offset: u64,
    length: usize,
    max_length: usize,
) -> io::Result<Vec<u8>> {
    if length > max_length || length > ABSOLUTE_MAX_RANGE {
        return Err(io::Error::other("object byte range exceeds limit"));
    }
    let (mut file, baseline) = open_verified_object(path)?;
    validate_range(offset, length, baseline.len())?;
    let bytes = read_at(&mut file, offset, length)?;
    verify_object_after_read(path, &file, &baseline)?;
    Ok(bytes)
}

/// Read several exact byte ranges using one file handle and a single memory budget.
/// All range bounds are checked before any bytes are returned.
pub fn read_object_ranges(
    path: &Path,
    ranges: &[(u64, usize)],
    max_total_bytes: usize,
) -> io::Result<Vec<Vec<u8>>> {
    if ranges.len() > MAX_RANGES {
        return Err(io::Error::other("too many object ranges"));
    }
    let mut total = 0usize;
    for &(_, length) in ranges {
        total = total
            .checked_add(length)
            .ok_or_else(|| io::Error::other("object range size overflow"))?;
        if total > max_total_bytes || total > ABSOLUTE_MAX_RANGE {
            return Err(io::Error::other("object ranges exceed byte limit"));
        }
    }
    let (mut file, baseline) = open_verified_object(path)?;
    for &(offset, length) in ranges {
        validate_range(offset, length, baseline.len())?;
    }
    if let [(offset, length)] = ranges {
        // A single range needs neither interval sorting nor a second copy.
        let data = read_at(&mut file, *offset, *length)?;
        verify_object_after_read(path, &file, &baseline)?;
        return Ok(vec![data]);
    }
    // Mark-driven scans often request already ordered, disjoint ranges.
    // Read these directly into their final buffers without a sorted index
    // vector or a second copy. Adjacent or overlapping ranges keep the
    // coalescing path below to preserve the reduced syscall count.
    let mut previous_end = None;
    let strictly_disjoint = ranges.iter().filter(|(_, len)| *len > 0)
        .all(|&(offset, length)| {
            let ordered = previous_end.is_none_or(|end| offset > end);
            previous_end = Some(offset + length as u64); // bounds checked above
            ordered
        });
    if strictly_disjoint {
        let result = ranges.iter()
            .map(|&(offset, length)| read_at(&mut file, offset, length))
            .collect::<io::Result<Vec<_>>>()?;
        verify_object_after_read(path, &file, &baseline)?;
        return Ok(result);
    }
    // Adjacent or overlapping logical ranges share one physical read.
    // This reduces seeks and small I/O operations for mark-driven scans.
    // Preserve the request order and return independent byte buffers.
    let mut ordered: Vec<(usize, u64, u64)> = ranges
        .iter()
        .enumerate()
        .filter_map(|(index, &(offset, length))| {
            (length > 0).then_some((index, offset, offset + length as u64))
        })
        .collect();
    ordered.sort_unstable_by_key(|(_, start, _)| *start);
    let mut output = vec![Vec::new(); ranges.len()];
    let mut cursor = 0;
    while cursor < ordered.len() {
        let start = ordered[cursor].1;
        let mut end = ordered[cursor].2;
        let mut tail = cursor + 1;
        while tail < ordered.len() && ordered[tail].1 <= end {
            end = end.max(ordered[tail].2);
            tail += 1;
        }
        let bytes = usize::try_from(end - start)
            .map_err(|_| io::Error::other("coalesced object range too large"))?;
        // Every merged interval is covered by the sum of requested lengths,
        // which was checked against the 8 MiB request budget above.
        if tail == cursor + 1 {
            // For an isolated interval the final output can own the read
            // buffer. There is no reason to copy it through a temporary.
            let index = ordered[cursor].0;
            let range = read_at(&mut file, start, bytes)?;
            output[index] = range;
        } else {
            let chunk = read_at(&mut file, start, bytes)?;
            for &(index, offset, item_end) in &ordered[cursor..tail] {
                let from = (offset - start) as usize;
                let to = (item_end - start) as usize;
                output[index] = chunk[from..to].to_vec();
            }
        }
        cursor = tail;
    }
    verify_object_after_read(path, &file, &baseline)?;
    Ok(output)
}

/// Read independently checksummed immutable object chunks in one bounded
/// call. Verify every chunk before returning any data to a replica consumer.
/// CRC32C detects accidental corruption; it does not authenticate peers.
pub fn read_object_ranges_checked(
    path: &Path,
    ranges: &[(u64, usize, u32)],
    max_total_bytes: usize,
) -> io::Result<Vec<Vec<u8>>> {
    if ranges.len() > MAX_RANGES {
        return Err(io::Error::other("too many object ranges"));
    }
    let spans = ranges.iter().map(|&(offset, length, _)| (offset, length))
        .collect::<Vec<_>>();
    let chunks = read_object_ranges(path, &spans, max_total_bytes)?;
    for (chunk, &(_, _, expected_crc32c)) in chunks.iter().zip(ranges) {
        if crate::distributed_crc32c::crc32c(chunk) != expected_crc32c {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "replica object range checksum mismatch",
            ));
        }
    }
    Ok(chunks)
}

/// Verify checksums of immutable byte ranges without buffering their contents.
///
/// Unlike read_object_ranges_checked, this scrub-only path retains one fixed
/// 64 KiB block regardless of total range length. Every offset, total byte
/// budget and range count is checked before any data is read.
pub fn verify_object_ranges_crc32c(
    path: &Path,
    ranges: &[(u64, u64, u32)],
    max_total_bytes: u64,
) -> io::Result<()> {
    const MAX_VERIFY_BYTES: u64 = 64 * 1024 * 1024;
    if ranges.len() > MAX_RANGES {
        return Err(io::Error::other("too many object checksum ranges"));
    }
    let mut requested = 0u64;
    for &(_, length, _) in ranges {
        requested = requested.checked_add(length)
            .ok_or_else(|| io::Error::other("object checksum range size overflow"))?;
        if requested > max_total_bytes || requested > MAX_VERIFY_BYTES {
            return Err(io::Error::other("object checksum ranges exceed byte limit"));
        }
    }
    let (mut file, before) = open_verified_object(path)?;
    for &(offset, length, _) in ranges {
        let end = offset.checked_add(length)
            .ok_or_else(|| io::Error::other("object checksum range overflow"))?;
        if end > before.len() {
            return Err(io::Error::other("object checksum range exceeds file length"));
        }
    }
    let mut buffer = [0u8; 64 * 1024];
    for &(offset, length, expected) in ranges {
        file.seek(SeekFrom::Start(offset))?;
        let mut remaining = length;
        let mut checksum = crate::distributed_crc32c::Crc32cState::new();
        while remaining > 0 {
            let amount = remaining.min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..amount])?;
            checksum.append(&buffer[..amount]);
            remaining -= amount as u64;
        }
        if checksum.checksum() != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData, "replica checksum range mismatch",
            ));
        }
    }
    verify_object_after_read(path, &file, &before)
}

#[cfg(test)]
mod streaming_checksum_range_tests {
    use super::*;

    #[test]
    fn validates_large_sparse_overlapping_ranges_without_retaining_bytes() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-stream-check-{}-{:?}", std::process::id(), std::thread::current().id()
        ));
        let bytes = (0..(512 * 1024)).map(|index| (index % 251) as u8).collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        let checksum = crate::distributed_crc32c::crc32c;
        let ranges = [
            (0, bytes.len() as u64, checksum(&bytes)),
            (1024, 65_537, checksum(&bytes[1024..1024 + 65_537])),
            (bytes.len() as u64, 0, checksum(b"")),
            (0, 1, checksum(&bytes[..1])),
        ];
        verify_object_ranges_crc32c(&path, &ranges, 700_000).unwrap();
        let mut bad = ranges;
        bad[1].2 ^= 1;
        assert_eq!(
            verify_object_ranges_crc32c(&path, &bad, 700_000).unwrap_err().kind(),
            io::ErrorKind::InvalidData,
        );
        assert!(verify_object_ranges_crc32c(&path, &ranges, 1024).is_err());
        assert!(verify_object_ranges_crc32c(&path, &[(u64::MAX, 2, 0)], u64::MAX).is_err());
        assert!(verify_object_ranges_crc32c(&path, &vec![(0, 0, 0); 1025], u64::MAX).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn streamed_crc_agrees_with_checked_range_reader_for_small_ranges() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-stream-compare-{}-{:?}", std::process::id(), std::thread::current().id()
        ));
        let bytes = (0..16_384).map(|i| (i * 13 % 251) as u8).collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        let mut ranges = Vec::new();
        for index in 0..48usize {
            let start = index * 241;
            let len = index % 193;
            let expected = crate::distributed_crc32c::crc32c(&bytes[start..start + len]);
            ranges.push((start as u64, len as u64, expected));
        }
        verify_object_ranges_crc32c(&path, &ranges, 48 * 193).unwrap();
        let read_ranges = ranges.iter().map(|&(offset, len, checksum)| {
            (offset, len as usize, checksum)
        }).collect::<Vec<_>>();
        assert!(read_object_ranges_checked(&path, &read_ranges, 48 * 193).is_ok());
        fs::remove_file(path).unwrap();
    }
}

#[cfg(all(test, unix))]
mod immutable_object_identity_tests {
    use super::*;

    #[test]
    fn rejects_an_unrelated_equal_length_inode() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-range-identity-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let a = root.join("a");
        let b = root.join("b");
        let link = root.join("hardlink");
        fs::write(&a, b"same").unwrap();
        fs::write(&b, b"size").unwrap();
        fs::hard_link(&a, &link).unwrap();
        assert!(same_object_identity(&fs::metadata(&a).unwrap(), &fs::metadata(&link).unwrap()));
        assert!(!same_object_identity(&fs::metadata(&a).unwrap(), &fs::metadata(&b).unwrap()));
        let baseline = fs::metadata(&a).unwrap();
        assert!(object_state_unchanged(&baseline, &baseline));
        assert!(!object_state_unchanged(&baseline, &fs::metadata(&b).unwrap()));
        fs::write(&a, b"longer").unwrap();
        assert!(!object_state_unchanged(&baseline, &fs::metadata(&a).unwrap()));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singleton_multi_range_reader_returns_exact_bytes() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-single-range-{}", std::process::id()
        ));
        fs::write(&path, b"abcdefgh").unwrap();
        assert_eq!(
            read_object_ranges(&path, &[(2, 4)], 4).unwrap(),
            vec![b"cdef".to_vec()]
        );
        assert_eq!(
            read_object_ranges(&path, &[(8, 0)], 0).unwrap(),
            vec![Vec::new()]
        );
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn reads_exact_intervals_and_rejects_bad_ranges() {
        let path = std::env::temp_dir().join(format!("rusthouse-range-{}", std::process::id()));
        fs::write(&path, b"0123456789").unwrap();
        assert_eq!(read_object_range(&path, 3, 4, 8).unwrap(), b"3456");
        assert_eq!(read_object_range(&path, 10, 0, 8).unwrap(), b"");
        assert!(read_object_range(&path, 9, 2, 8).is_err());
        assert!(read_object_range(&path, u64::MAX, 2, 8).is_err());
        assert!(read_object_range(&path, 0, 9, 8).is_err());
        fs::remove_file(path).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_range_sources() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("rusthouse-range-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let original = dir.join("object");
        let link = dir.join("link");
        fs::write(&original, b"abc").unwrap();
        symlink(&original, &link).unwrap();
        assert!(read_object_range(&link, 0, 1, 2).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod multi_range_tests {
    use super::*;

    #[test]
    fn overlapping_and_adjacent_ranges_preserve_request_order() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-overlap-ranges-{}", std::process::id()
        ));
        fs::write(&path, b"abcdefghijklmnop").unwrap();
        let selections = [(8, 4), (0, 4), (2, 5), (7, 1), (8, 0), (14, 2)];
        let ranges = read_object_ranges(&path, &selections, 17).unwrap();
        assert_eq!(ranges, vec![
            b"ijkl".to_vec(), b"abcd".to_vec(), b"cdefg".to_vec(),
            b"h".to_vec(), Vec::new(), b"op".to_vec(),
        ]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn disjoint_large_ranges_return_exact_data_in_request_order() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-disjoint-ranges-{}", std::process::id()
        ));
        let bytes = (0..256 * 1024)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        let ranges = [(128 * 1024, 64 * 1024), (0, 32 * 1024), (224 * 1024, 16 * 1024)];
        let result = read_object_ranges(&path, &ranges, 112 * 1024).unwrap();
        for (index, &(start, len)) in ranges.iter().enumerate() {
            assert_eq!(&result[index], &bytes[start as usize..start as usize + len]);
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_multiple_intervals_in_request_order() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-multi-range-{}", std::process::id()
        ));
        fs::write(&path, b"0123456789").unwrap();
        assert_eq!(
            read_object_ranges(&path, &[(8, 2), (0, 3), (5, 0)], 5).unwrap(),
            vec![b"89".to_vec(), b"012".to_vec(), Vec::new()]
        );
        assert!(read_object_ranges(&path, &[(0, 3), (8, 2)], 4).is_err());
        assert!(read_object_ranges(&path, &[(9, 2)], 8).is_err());
        assert!(read_object_ranges(&path, &[(u64::MAX, 2)], 8).is_err());
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod zero_offset_object_ranges_tests {
    use super::*;

    #[test]
    fn reads_prefix_and_nonzero_offsets_in_both_range_apis() {
        let path = std::env::temp_dir().join(format!("rusthouse-range-zero-seek-{}", std::process::id()));
        fs::write(&path, b"abcdefghijkl").unwrap();
        assert_eq!(read_object_range(&path, 0, 4, 12).unwrap(), b"abcd");
        assert_eq!(read_object_range(&path, 4, 4, 12).unwrap(), b"efgh");
        assert_eq!(read_object_ranges(&path, &[(0, 4)], 4).unwrap(), vec![b"abcd".to_vec()]);
        assert_eq!(read_object_ranges(&path, &[(8, 2), (0, 3)], 5).unwrap(), vec![b"ij".to_vec(), b"abc".to_vec()]);
        assert_eq!(read_object_ranges(&path, &[(0, 6), (3, 6)], 12).unwrap(), vec![b"abcdef".to_vec(), b"defghi".to_vec()]);
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod checked_multi_range_tests {
    use super::*;

    #[test]
    fn verifies_overlapping_chunk_checksums_and_rejects_corruption() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-checked-range-{}", std::process::id()
        ));
        let bytes = b"the quick brown fox jumped over a lazy dog";
        fs::write(&path, bytes).unwrap();
        let checksum = crate::distributed_crc32c::crc32c;
        let ranges = vec![
            (0, 9, checksum(&bytes[0..9])),
            (4, 16, checksum(&bytes[4..20])),
            (bytes.len() as u64, 0, checksum(b"")),
            (2, 4, checksum(&bytes[2..6])),
        ];
        let result = read_object_ranges_checked(&path, &ranges, 32).unwrap();
        assert_eq!(result, vec![
            bytes[0..9].to_vec(), bytes[4..20].to_vec(),
            Vec::new(), bytes[2..6].to_vec(),
        ]);
        let mut corrupted = ranges.clone();
        corrupted[1].2 ^= 1;
        assert_eq!(
            read_object_ranges_checked(&path, &corrupted, 32)
                .unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(read_object_ranges_checked(&path, &ranges, 8).is_err());
        fs::remove_file(path).unwrap();
    }
}

#[cfg(all(test, unix))]
mod positioned_range_tests {
    use super::*;
    use std::io::{Seek, SeekFrom};

    #[test]
    fn positioned_reads_preserve_cursor_and_match_ordinary_seeks() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-positioned-ranges-{}-{}", std::process::id(), line!()
        ));
        let input = (0..256 * 1024).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        fs::write(&path, &input).unwrap();
        let mut file = File::open(&path).unwrap();
        file.seek(SeekFrom::Start(71)).unwrap();
        for (start, length) in [(0, 5), (80_000, 77), (17, 4096), (250_000, 144), (256 * 1024, 0)] {
            assert_eq!(read_at(&mut file, start, length).unwrap(), &input[start as usize..start as usize + length]);
            assert_eq!(file.stream_position().unwrap(), 71);
        }
        assert_eq!(read_at(&mut file, 256 * 1024, 1).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(file.stream_position().unwrap(), 71);
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod range_open_metadata_consistency_tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn a_same_size_overwrite_is_not_a_stable_immutable_object() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-range-open-metadata-{}-{:?}",
            std::process::id(), std::thread::current().id()
        ));
        fs::write(&path, b"original").unwrap();
        let before = fs::symlink_metadata(&path).unwrap();
        // Force a distinct timestamp even on filesystems with coarse clocks.
        let file = File::options().write(true).open(&path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)
        )).unwrap();
        let after = file.metadata().unwrap();
        assert_eq!(before.len(), after.len());
        assert!(!object_state_unchanged(&before, &after));
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod direct_disjoint_range_read_tests {
    use super::*;

    #[test]
    fn ordered_sparse_ranges_and_zero_lengths_match_exact_file_slices() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-range-disjoint-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("part.bin");
        let data = (0..8192usize).map(|n| (n * 31 % 256) as u8).collect::<Vec<_>>();
        fs::write(&path, &data).unwrap();
        for ranges in [
            vec![(0, 32), (64, 32), (200, 800), (4000, 100), (8100, 92)],
            vec![(0, 0), (1, 17), (1, 0), (64, 200), (8192, 0)],
            vec![(50, 10), (60, 10), (70, 10)],
            vec![(400, 50), (425, 100), (400, 50), (10, 10)],
        ] {
            let total = ranges.iter().map(|(_, length)| *length).sum::<usize>();
            let actual = read_object_ranges(&path, &ranges, total).unwrap();
            let expected = ranges.iter().map(|&(offset, length)| {
                data[offset as usize..offset as usize + length].to_vec()
            }).collect::<Vec<_>>();
            assert_eq!(actual, expected, "ranges={ranges:?}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sorted_disjoint_reads_still_check_total_budget_and_bounds() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-range-bounds-{}-{:?}", std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("data");
        fs::write(&path, vec![7u8; 256]).unwrap();
        assert!(read_object_ranges(&path, &[(0, 4), (20, 4)], 7).is_err());
        assert!(read_object_ranges(&path, &[(0, 4), (250, 7)], 100).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

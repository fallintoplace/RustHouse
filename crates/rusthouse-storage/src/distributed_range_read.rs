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
        same_object_identity(before, after)
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }
    #[cfg(not(unix))]
    true
}

fn verify_object_after_read(path: &Path, file: &File, before: &fs::Metadata) -> io::Result<()> {
    let end_fd = file.metadata()?;
    let end_path = fs::symlink_metadata(path)?;
    if !object_state_unchanged(before, &end_fd) || !object_state_unchanged(before, &end_path) {
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

fn read_at_into(file: &mut File, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(bytes, offset)?;
    }
    #[cfg(not(unix))]
    {
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(bytes)?;
    }
    Ok(())
}

fn read_at(file: &mut File, offset: u64, length: usize) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; length];
    read_at_into(file, offset, &mut bytes)?;
    Ok(bytes)
}

/// Read one verified immutable range into an existing caller-owned buffer.
///
/// This avoids allocating and copying another buffer for repeated replica
/// pulls. On error, the destination may have been partially modified and
/// must not be used.
pub fn read_object_range_into(
    path: &Path,
    offset: u64,
    destination: &mut [u8],
    max_length: usize,
) -> io::Result<()> {
    if destination.len() > max_length || destination.len() > ABSOLUTE_MAX_RANGE {
        return Err(io::Error::other("object byte range exceeds limit"));
    }
    let (mut file, baseline) = open_verified_object(path)?;
    validate_range(offset, destination.len(), baseline.len())?;
    read_at_into(&mut file, offset, destination)?;
    verify_object_after_read(path, &file, &baseline)
}

// A handful of out-of-order mark ranges can be proven disjoint with
// stack-only pair checks. Avoid sorting an index vector for these requests.
fn small_scattered_disjoint(ranges: &[(u64, usize)]) -> bool {
    ranges.len() <= 8
        && ranges.iter().enumerate().all(|(index, &(start, length))| {
            length == 0
                || ranges[..index].iter().all(|&(other_start, other_length)| {
                    other_length == 0
                        || start > other_start + other_length as u64
                        || other_start > start + length as u64
                })
        })
}

/// Read a batch of immutable object ranges into caller-owned buffers.
///
/// All requests are validated before the first destination is modified.
/// No per-range Vec is allocated, and one verified file handle is retained
/// across every positioned read. On an I/O or identity error, destinations
/// may be partially modified and must not be used.
pub fn read_object_ranges_into(
    path: &Path,
    ranges: &mut [(u64, &mut [u8])],
    max_total_bytes: usize,
) -> io::Result<()> {
    if ranges.len() > MAX_RANGES {
        return Err(io::Error::other("too many object ranges"));
    }
    let mut total = 0usize;
    for (_, destination) in ranges.iter() {
        total = total
            .checked_add(destination.len())
            .ok_or_else(|| io::Error::other("object range size overflow"))?;
        if total > max_total_bytes || total > ABSOLUTE_MAX_RANGE {
            return Err(io::Error::other("object ranges exceed byte limit"));
        }
    }
    let (mut file, baseline) = open_verified_object(path)?;
    for (offset, destination) in ranges.iter() {
        validate_range(*offset, destination.len(), baseline.len())?;
    }
    for (offset, destination) in ranges.iter_mut() {
        read_at_into(&mut file, *offset, destination)?;
    }
    verify_object_after_read(path, &file, &baseline)
}

#[cfg(test)]
mod direct_batched_range_buffers_tests {
    use super::*;

    #[test]
    fn unordered_and_overlapping_ranges_fill_exact_caller_buffers() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-batch-range-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let contents = (0..=255u8).collect::<Vec<_>>();
        fs::write(&path, &contents).unwrap();
        let mut first = [0u8; 17];
        let mut second = [0u8; 7];
        let mut overlap = [0u8; 10];
        let mut empty = [];
        let mut ranges = [
            (100u64, first.as_mut_slice()),
            (3, second.as_mut_slice()),
            (105, overlap.as_mut_slice()),
            (256, empty.as_mut_slice()),
        ];
        read_object_ranges_into(&path, &mut ranges, 34).unwrap();
        assert_eq!(first.as_slice(), &contents[100..117]);
        assert_eq!(second.as_slice(), &contents[3..10]);
        assert_eq!(overlap.as_slice(), &contents[105..115]);

        let mut unchanged = [0xcc; 8];
        let mut ok = [0xaa; 2];
        {
            let mut invalid = [
                (0, ok.as_mut_slice()),
                (252, unchanged.as_mut_slice()),
            ];
            assert!(read_object_ranges_into(&path, &mut invalid, 10).is_err());
            assert!(read_object_ranges_into(&path, &mut invalid, 9).is_err());
        }
        assert_eq!(ok, [0xaa; 2]);
        assert_eq!(unchanged, [0xcc; 8]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn zero_ranges_keep_identity_validation_and_size_limits() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-batch-empty-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::write(&path, [1u8, 2, 3]).unwrap();
        let mut requests: [(u64, &mut [u8]); 0] = [];
        read_object_ranges_into(&path, &mut requests, 0).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(read_object_ranges_into(&path, &mut requests, 0).is_err());
    }
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
    let strictly_disjoint = ranges
        .iter()
        .filter(|(_, len)| *len > 0)
        .all(|&(offset, length)| {
            let ordered = previous_end.is_none_or(|end| offset > end);
            previous_end = Some(offset + length as u64); // bounds checked above
            ordered
        });
    if strictly_disjoint || small_scattered_disjoint(ranges) {
        let result = ranges
            .iter()
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
    if let [(offset, length, expected_crc32c)] = ranges {
        // A single verified chunk needs no intermediate span vector. The
        // ordinary range reader enforces the same byte and file limits.
        let chunk = read_object_range(path, *offset, *length, max_total_bytes)?;
        if crate::distributed_crc32c::crc32c(&chunk) != *expected_crc32c {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "replica object range checksum mismatch",
            ));
        }
        return Ok(vec![chunk]);
    }
    // Keep up to eight checked range spans in a stack array rather than
    // allocating a Vec for small read batches.
    const INLINE_RANGES: usize = 8;
    let mut inline = [(0u64, 0usize); INLINE_RANGES];
    let owned = (ranges.len() > INLINE_RANGES).then(|| {
        ranges
            .iter()
            .map(|&(offset, length, _)| (offset, length))
            .collect::<Vec<_>>()
    });
    let spans: &[(u64, usize)] = if let Some(ref spans) = owned {
        spans
    } else {
        for (slot, &(offset, length, _)) in inline.iter_mut().zip(ranges) {
            *slot = (offset, length);
        }
        &inline[..ranges.len()]
    };
    let chunks = read_object_ranges(path, spans, max_total_bytes)?;
    // Coalesced reads already avoid duplicate I/O, but independently hashing
    // the returned copies can redo megabytes of CRC work for repeated spans.
    // Keep the small-request path allocation-free.
    let mut cached = (ranges.len() >= 32)
        .then(|| std::collections::HashMap::<(u64, usize), u32>::with_capacity(ranges.len()));
    for (chunk, &(offset, length, expected_crc32c)) in chunks.iter().zip(ranges) {
        let actual = if let Some(cache) = &mut cached {
            *cache
                .entry((offset, length))
                .or_insert_with(|| crate::distributed_crc32c::crc32c(chunk))
        } else {
            crate::distributed_crc32c::crc32c(chunk)
        };
        if actual != expected_crc32c {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "replica object range checksum mismatch",
            ));
        }
    }
    Ok(chunks)
}

/// Use positioned reads on Unix, preserving the same verification behavior
/// on other platforms without retaining any requested range in memory.
fn read_scrub_chunk(file: &mut File, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buffer, offset)
    }
    #[cfg(not(unix))]
    {
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buffer)
    }
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
        requested = requested
            .checked_add(length)
            .ok_or_else(|| io::Error::other("object checksum range size overflow"))?;
        if requested > max_total_bytes || requested > MAX_VERIFY_BYTES {
            return Err(io::Error::other("object checksum ranges exceed byte limit"));
        }
    }
    // A large scrub request can repeat the same logical range many times.
    // Count every request against the original budget, but hash each unique
    // physical span only once. Conflicting expected checksums fail closed.
    if ranges.len() >= 32 {
        let mut by_span = std::collections::HashMap::with_capacity(ranges.len());
        let mut unique = Vec::with_capacity(ranges.len());
        for &(offset, length, expected) in ranges {
            match by_span.insert((offset, length), expected) {
                Some(previous) if previous != expected => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "conflicting checksums for the same object range",
                    ));
                }
                Some(_) => {}
                None => unique.push((offset, length, expected)),
            }
        }
        if unique.len() != ranges.len() {
            // The recursive call has strictly fewer spans, so no repeated
            // deduplication work is required; all checks remain identical.
            return verify_object_ranges_crc32c(path, &unique, max_total_bytes);
        }
    }
    let (mut file, before) = open_verified_object(path)?;
    for &(offset, length, _) in ranges {
        let end = offset
            .checked_add(length)
            .ok_or_else(|| io::Error::other("object checksum range overflow"))?;
        if end > before.len() {
            return Err(io::Error::other(
                "object checksum range exceeds file length",
            ));
        }
    }
    let mut buffer = [0u8; 64 * 1024];
    // Already-ordered disjoint scrub ranges need no sorting or active-state
    // bookkeeping. Preserve that fast path for ordinary sequential copies.
    let mut previous_end = None;
    let ordered_disjoint =
        ranges
            .iter()
            .filter(|(_, length, _)| *length > 0)
            .all(|&(offset, length, _)| {
                let sorted = previous_end.is_none_or(|end| offset >= end);
                previous_end = Some(offset + length); // checked against file bounds
                sorted
            });

    if ordered_disjoint {
        for &(offset, length, expected) in ranges {
            let mut position = offset;
            let mut remaining = length;
            let mut checksum = crate::distributed_crc32c::Crc32cState::new();
            while remaining > 0 {
                let amount = remaining.min(buffer.len() as u64) as usize;
                read_scrub_chunk(&mut file, position, &mut buffer[..amount])?;
                checksum.append(&buffer[..amount]);
                position += amount as u64;
                remaining -= amount as u64;
            }
            if checksum.checksum() != expected {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "replica checksum range mismatch",
                ));
            }
        }
    } else {
        // For overlapping requests, read each physical byte once and update
        // every live checksum for that segment. Memory is bounded by one 64 KiB
        // I/O buffer, 1024 checksum states, and 1024 active interval indices.
        let mut intervals: Vec<(u64, u64, usize)> = ranges
            .iter()
            .enumerate()
            .filter_map(|(index, &(offset, length, _))| {
                (length > 0).then_some((offset, offset + length, index))
            })
            .collect();
        intervals.sort_unstable_by_key(|entry| entry.0);
        let mut states = (0..ranges.len())
            .map(|_| crate::distributed_crc32c::Crc32cState::new())
            .collect::<Vec<_>>();
        let mut active = Vec::<usize>::new();
        let mut next = 0usize;
        let mut position = 0u64;
        while next < intervals.len() || !active.is_empty() {
            if active.is_empty() {
                position = intervals[next].0;
            }
            while next < intervals.len() && intervals[next].0 == position {
                active.push(intervals[next].2);
                next += 1;
            }
            let next_start = intervals.get(next).map_or(u64::MAX, |entry| entry.0);
            let next_end = active
                .iter()
                .map(|&index| ranges[index].0 + ranges[index].1)
                .min()
                .unwrap_or(u64::MAX);
            let boundary = next_start
                .min(next_end)
                .min(position.saturating_add(buffer.len() as u64));
            let amount = (boundary - position) as usize;
            read_scrub_chunk(&mut file, position, &mut buffer[..amount])?;
            for &index in &active {
                states[index].append(&buffer[..amount]);
            }
            position = boundary;
            active.retain(|&index| ranges[index].0 + ranges[index].1 > position);
        }
        for ((_, _, expected), checksum) in ranges.iter().zip(&states) {
            if checksum.checksum() != *expected {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "replica checksum range mismatch",
                ));
            }
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
            "rusthouse-stream-check-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let bytes = (0..(512 * 1024))
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
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
            verify_object_ranges_crc32c(&path, &bad, 700_000)
                .unwrap_err()
                .kind(),
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
            "rusthouse-stream-compare-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let bytes = (0..16_384)
            .map(|i| (i * 13 % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        let mut ranges = Vec::new();
        for index in 0..48usize {
            let start = index * 241;
            let len = index % 193;
            let expected = crate::distributed_crc32c::crc32c(&bytes[start..start + len]);
            ranges.push((start as u64, len as u64, expected));
        }
        verify_object_ranges_crc32c(&path, &ranges, 48 * 193).unwrap();
        let read_ranges = ranges
            .iter()
            .map(|&(offset, len, checksum)| (offset, len as usize, checksum))
            .collect::<Vec<_>>();
        assert!(read_object_ranges_checked(&path, &read_ranges, 48 * 193).is_ok());
        fs::remove_file(path).unwrap();
    }
}

#[cfg(all(test, unix))]
mod immutable_object_identity_tests {
    use super::*;

    #[test]
    fn rejects_an_unrelated_equal_length_inode() {
        let root =
            std::env::temp_dir().join(format!("rusthouse-range-identity-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let a = root.join("a");
        let b = root.join("b");
        let link = root.join("hardlink");
        fs::write(&a, b"same").unwrap();
        fs::write(&b, b"size").unwrap();
        fs::hard_link(&a, &link).unwrap();
        assert!(same_object_identity(
            &fs::metadata(&a).unwrap(),
            &fs::metadata(&link).unwrap()
        ));
        assert!(!same_object_identity(
            &fs::metadata(&a).unwrap(),
            &fs::metadata(&b).unwrap()
        ));
        let baseline = fs::metadata(&a).unwrap();
        assert!(object_state_unchanged(&baseline, &baseline));
        assert!(!object_state_unchanged(
            &baseline,
            &fs::metadata(&b).unwrap()
        ));
        fs::write(&a, b"longer").unwrap();
        assert!(!object_state_unchanged(
            &baseline,
            &fs::metadata(&a).unwrap()
        ));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singleton_multi_range_reader_returns_exact_bytes() {
        let path =
            std::env::temp_dir().join(format!("rusthouse-single-range-{}", std::process::id()));
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
        let path =
            std::env::temp_dir().join(format!("rusthouse-overlap-ranges-{}", std::process::id()));
        fs::write(&path, b"abcdefghijklmnop").unwrap();
        let selections = [(8, 4), (0, 4), (2, 5), (7, 1), (8, 0), (14, 2)];
        let ranges = read_object_ranges(&path, &selections, 17).unwrap();
        assert_eq!(
            ranges,
            vec![
                b"ijkl".to_vec(),
                b"abcd".to_vec(),
                b"cdefg".to_vec(),
                b"h".to_vec(),
                Vec::new(),
                b"op".to_vec(),
            ]
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn disjoint_large_ranges_return_exact_data_in_request_order() {
        let path =
            std::env::temp_dir().join(format!("rusthouse-disjoint-ranges-{}", std::process::id()));
        let bytes = (0..256 * 1024)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        let ranges = [
            (128 * 1024, 64 * 1024),
            (0, 32 * 1024),
            (224 * 1024, 16 * 1024),
        ];
        let result = read_object_ranges(&path, &ranges, 112 * 1024).unwrap();
        for (index, &(start, len)) in ranges.iter().enumerate() {
            assert_eq!(&result[index], &bytes[start as usize..start as usize + len]);
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_multiple_intervals_in_request_order() {
        let path =
            std::env::temp_dir().join(format!("rusthouse-multi-range-{}", std::process::id()));
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
        let path =
            std::env::temp_dir().join(format!("rusthouse-range-zero-seek-{}", std::process::id()));
        fs::write(&path, b"abcdefghijkl").unwrap();
        assert_eq!(read_object_range(&path, 0, 4, 12).unwrap(), b"abcd");
        assert_eq!(read_object_range(&path, 4, 4, 12).unwrap(), b"efgh");
        assert_eq!(
            read_object_ranges(&path, &[(0, 4)], 4).unwrap(),
            vec![b"abcd".to_vec()]
        );
        assert_eq!(
            read_object_ranges(&path, &[(8, 2), (0, 3)], 5).unwrap(),
            vec![b"ij".to_vec(), b"abc".to_vec()]
        );
        assert_eq!(
            read_object_ranges(&path, &[(0, 6), (3, 6)], 12).unwrap(),
            vec![b"abcdef".to_vec(), b"defghi".to_vec()]
        );
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod checked_multi_range_tests {
    use super::*;

    #[test]
    fn verifies_overlapping_chunk_checksums_and_rejects_corruption() {
        let path =
            std::env::temp_dir().join(format!("rusthouse-checked-range-{}", std::process::id()));
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
        assert_eq!(
            result,
            vec![
                bytes[0..9].to_vec(),
                bytes[4..20].to_vec(),
                Vec::new(),
                bytes[2..6].to_vec(),
            ]
        );
        let mut corrupted = ranges.clone();
        corrupted[1].2 ^= 1;
        assert_eq!(
            read_object_ranges_checked(&path, &corrupted, 32)
                .unwrap_err()
                .kind(),
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
            "rusthouse-positioned-ranges-{}-{}",
            std::process::id(),
            line!()
        ));
        let input = (0..256 * 1024).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        fs::write(&path, &input).unwrap();
        let mut file = File::open(&path).unwrap();
        file.seek(SeekFrom::Start(71)).unwrap();
        for (start, length) in [
            (0, 5),
            (80_000, 77),
            (17, 4096),
            (250_000, 144),
            (256 * 1024, 0),
        ] {
            assert_eq!(
                read_at(&mut file, start, length).unwrap(),
                &input[start as usize..start as usize + length]
            );
            assert_eq!(file.stream_position().unwrap(), 71);
        }
        assert_eq!(
            read_at(&mut file, 256 * 1024, 1).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
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
            std::process::id(),
            std::thread::current().id()
        ));
        fs::write(&path, b"original").unwrap();
        let before = fs::symlink_metadata(&path).unwrap();
        // Force a distinct timestamp even on filesystems with coarse clocks.
        let file = File::options().write(true).open(&path).unwrap();
        file.set_times(
            std::fs::FileTimes::new()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)),
        )
        .unwrap();
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
        let data = (0..8192usize)
            .map(|n| (n * 31 % 256) as u8)
            .collect::<Vec<_>>();
        fs::write(&path, &data).unwrap();
        for ranges in [
            vec![(0, 32), (64, 32), (200, 800), (4000, 100), (8100, 92)],
            vec![(0, 0), (1, 17), (1, 0), (64, 200), (8192, 0)],
            vec![(50, 10), (60, 10), (70, 10)],
            vec![(400, 50), (425, 100), (400, 50), (10, 10)],
        ] {
            let total = ranges.iter().map(|(_, length)| *length).sum::<usize>();
            let actual = read_object_ranges(&path, &ranges, total).unwrap();
            let expected = ranges
                .iter()
                .map(|&(offset, length)| data[offset as usize..offset as usize + length].to_vec())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "ranges={ranges:?}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sorted_disjoint_reads_still_check_total_budget_and_bounds() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-range-bounds-{}-{:?}",
            std::process::id(),
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

#[cfg(test)]
mod positioned_scrub_regression_tests {
    use super::*;

    #[test]
    fn unaligned_out_of_order_and_overlapping_scrubs_match_reference_crc() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-positioned-scrub-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_file(&root);
        let bytes = (0..(256 * 1024 + 17))
            .map(|index| ((index * 67 + index / 13) % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&root, &bytes).unwrap();
        let ranges = [
            (120_011usize, 80_003usize),
            (3, 190_000),
            (bytes.len(), 0),
            (70_017, 65_537),
            (1, 1),
            (0, bytes.len()),
        ];
        let checksummed = ranges.map(|(start, length)| {
            (
                start as u64,
                length as u64,
                crate::distributed_crc32c::crc32c(&bytes[start..start + length]),
            )
        });
        let budget = checksummed.iter().map(|(_, length, _)| *length).sum();
        verify_object_ranges_crc32c(&root, &checksummed, budget).unwrap();
        let mut invalid = checksummed;
        invalid[3].2 ^= 0x4000;
        assert_eq!(
            verify_object_ranges_crc32c(&root, &invalid, budget)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_file(root).unwrap();
    }
}

#[cfg(test)]
mod single_checked_range_fast_path_tests {
    use super::*;

    #[test]
    fn single_checked_reads_preserve_bounds_empty_ranges_and_crc_contract() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-checked-single-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let bytes = (0u32..256).map(|n| (n % 251) as u8).collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        let checksum = crate::distributed_crc32c::crc32c;
        let correct = (17, 64, checksum(&bytes[17..81]));
        assert_eq!(
            read_object_ranges_checked(&path, &[correct], 64).unwrap(),
            vec![bytes[17..81].to_vec()]
        );
        assert_eq!(
            read_object_ranges_checked(&path, &[(bytes.len() as u64, 0, checksum(b""))], 0)
                .unwrap(),
            vec![Vec::<u8>::new()]
        );
        let bad = (17, 64, correct.2 ^ 1);
        assert_eq!(
            read_object_ranges_checked(&path, &[bad], 64)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(read_object_ranges_checked(&path, &[correct], 63).is_err());
        assert!(read_object_ranges_checked(&path, &[(250, 7, 0)], 64).is_err());
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod small_scattered_disjoint_tests {
    use super::*;

    #[test]
    fn keeps_scattered_disjoint_requests_on_the_direct_read_path() {
        assert!(small_scattered_disjoint(&[
            (15, 3),
            (0, 2),
            (8, 4),
            (21, 0)
        ]));
        assert!(!small_scattered_disjoint(&[(15, 3), (17, 3)]));
        assert!(!small_scattered_disjoint(&[(10, 2), (12, 3)]));
        assert!(!small_scattered_disjoint(&[(0, 1); 9]));
    }

    #[test]
    fn reads_out_of_order_small_ranges_in_original_order() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-scattered-range-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
        let spans = [(20, 3), (0, 3), (10, 2), (6, 0)];
        let actual = read_object_ranges(&path, &spans, 8).unwrap();
        assert_eq!(
            actual,
            vec![b"uvw".to_vec(), b"abc".to_vec(), b"kl".to_vec(), Vec::new()]
        );
        let overlapping = read_object_ranges(&path, &[(4, 4), (6, 4)], 8).unwrap();
        assert_eq!(overlapping, vec![b"efgh".to_vec(), b"ghij".to_vec()]);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod inline_checked_range_tests {
    use super::*;

    #[test]
    fn inline_and_heap_range_paths_preserve_order_and_individual_checksums() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-checked-ranges-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("object.bin");
        let bytes = (0..4096usize).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        std::fs::write(&path, &bytes).unwrap();
        for count in [0usize, 1, 2, 8, 9, 16] {
            let ranges = (0..count)
                .map(|index| {
                    let offset = ((count - index) * 7) as u64;
                    let len = 23usize;
                    let crc = crate::distributed_crc32c::crc32c(
                        &bytes[offset as usize..offset as usize + len],
                    );
                    (offset, len, crc)
                })
                .collect::<Vec<_>>();
            let actual = read_object_ranges_checked(&path, &ranges, 4096).unwrap();
            assert_eq!(actual.len(), count);
            for (result, &(offset, len, _)) in actual.iter().zip(&ranges) {
                assert_eq!(result, &bytes[offset as usize..offset as usize + len]);
            }
            if !ranges.is_empty() {
                let mut corrupt = ranges;
                corrupt[0].2 ^= 1;
                assert!(read_object_ranges_checked(&path, &corrupt, 4096).is_err());
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod coalesced_scrub_crc_tests {
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
}

#[cfg(test)]
mod in_place_range_tests {
    use super::*;

    #[test]
    fn existing_buffers_match_owned_exact_reads_at_unaligned_offsets() {
        let directory = std::env::temp_dir().join(format!(
            "rusthouse-inplace-range-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("object.bin");
        let source = (0..16384u32)
            .map(|value| (value % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&path, &source).unwrap();
        for &(offset, length) in &[(0, 0), (0, 1), (3, 7), (4095, 4096), (16384, 0)] {
            let mut destination = vec![0xcc; length];
            read_object_range_into(&path, offset, &mut destination, length).unwrap();
            assert_eq!(
                destination,
                read_object_range(&path, offset, length, length).unwrap()
            );
        }
        let mut untouched = [0xabu8; 16];
        assert!(read_object_range_into(&path, 16380, &mut untouched, 16).is_err());
        assert_eq!(untouched, [0xab; 16]);
        assert!(read_object_range_into(&path, 0, &mut untouched, 15).is_err());
        assert_eq!(untouched, [0xab; 16]);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod duplicate_crc_scrub_tests {
    use super::*;

    #[test]
    fn repeated_ranges_use_the_same_verification_and_reject_conflicts() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-duplicate-crc-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let bytes = (0..65_536usize)
            .map(|index| (index * 71 % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        let crc = crate::distributed_crc32c::crc32c(&bytes);
        let mut ranges = vec![(0u64, bytes.len() as u64, crc); 1024];
        verify_object_ranges_crc32c(&path, &ranges, 64 * 1024 * 1024).unwrap();

        ranges[511].2 ^= 1;
        assert_eq!(
            verify_object_ranges_crc32c(&path, &ranges, 64 * 1024 * 1024)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        ranges[511].2 = crc;
        assert!(verify_object_ranges_crc32c(&path, &ranges, 1024).is_err());

        let mut corrupted = bytes;
        corrupted[1234] ^= 1;
        fs::write(&path, &corrupted).unwrap();
        assert_eq!(
            verify_object_ranges_crc32c(&path, &ranges, 64 * 1024 * 1024)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod checked_duplicate_crc_cache_tests {
    use super::*;

    #[test]
    fn repeated_ranges_preserve_output_and_detect_conflicting_checksums() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-checked-duplicate-{}-{}",
            std::process::id(),
            line!()
        ));
        let bytes = (0..131_072usize)
            .map(|index| ((index * 37 + 11) % 251) as u8)
            .collect::<Vec<_>>();
        std::fs::write(&path, &bytes).unwrap();

        let spans = [(0u64, 4096usize), (21, 8000), (65_536, 32_768)];
        let requests = (0..64)
            .map(|index| {
                let (offset, length) = spans[index % spans.len()];
                let checksum = crate::distributed_crc32c::crc32c(
                    &bytes[offset as usize..offset as usize + length],
                );
                (offset, length, checksum)
            })
            .collect::<Vec<_>>();
        let output = read_object_ranges_checked(&path, &requests, 2 * 1024 * 1024).unwrap();
        for (actual, &(offset, length, _)) in output.iter().zip(&requests) {
            assert_eq!(actual, &bytes[offset as usize..offset as usize + length]);
        }

        let mut corrupted = requests.clone();
        corrupted[47].2 ^= 1;
        assert!(matches!(
            read_object_ranges_checked(&path, &corrupted, 2 * 1024 * 1024),
            Err(ref error) if error.kind() == io::ErrorKind::InvalidData
        ));
        std::fs::remove_file(path).unwrap();
    }
}

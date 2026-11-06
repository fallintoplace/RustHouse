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
    // Windows metadata does not expose stable file identity in Rust 1.85.
    // Creation time is an additional replacement check when the old and new
    // files happen to have the same length and last-write timestamp.
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        before.creation_time() == after.creation_time()
    }
    #[cfg(not(any(unix, windows)))]
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
    // Keep immutable objects pinned against in-place writes and replacement
    // on Windows for the entire verified range read.
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)?
    };
    #[cfg(not(windows))]
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

#[cfg(all(test, windows))]
mod windows_creation_time_tests {
    use super::*;

    #[test]
    fn detects_different_creation_time_despite_matching_modified_time() {
        let directory = std::env::temp_dir().join(format!(
            "rusthouse-window-range-creation-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first");
        let second = directory.join("second");
        fs::write(&first, b"abcd").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(&second, b"wxyz").unwrap();
        let old = fs::metadata(&first).unwrap();
        let writer = fs::OpenOptions::new().write(true).open(&second).unwrap();
        writer
            .set_times(fs::FileTimes::new().set_modified(old.modified().unwrap()))
            .unwrap();
        let new = fs::metadata(&second).unwrap();
        use std::os::windows::fs::MetadataExt;
        if old.creation_time() != new.creation_time() {
            assert!(!object_state_unchanged(&old, &new));
        }
        fs::remove_dir_all(&directory).unwrap();
    }
}

#[cfg(test)]
#[path = "distributed_range_read_tests/direct_batched_range_buffers_tests.rs"]
mod direct_batched_range_buffers_tests;

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
#[path = "distributed_range_read_tests/streaming_checksum_range_tests.rs"]
mod streaming_checksum_range_tests;

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
#[path = "distributed_range_read_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/multi_range_tests.rs"]
mod multi_range_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/zero_offset_object_ranges_tests.rs"]
mod zero_offset_object_ranges_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/checked_multi_range_tests.rs"]
mod checked_multi_range_tests;

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
#[path = "distributed_range_read_tests/range_open_metadata_consistency_tests.rs"]
mod range_open_metadata_consistency_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/direct_disjoint_range_read_tests.rs"]
mod direct_disjoint_range_read_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/positioned_scrub_regression_tests.rs"]
mod positioned_scrub_regression_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/single_checked_range_fast_path_tests.rs"]
mod single_checked_range_fast_path_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/small_scattered_disjoint_tests.rs"]
mod small_scattered_disjoint_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/inline_checked_range_tests.rs"]
mod inline_checked_range_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/coalesced_scrub_crc_tests.rs"]
mod coalesced_scrub_crc_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/in_place_range_tests.rs"]
mod in_place_range_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/duplicate_crc_scrub_tests.rs"]
mod duplicate_crc_scrub_tests;

#[cfg(test)]
#[path = "distributed_range_read_tests/checked_duplicate_crc_cache_tests.rs"]
mod checked_duplicate_crc_cache_tests;

#[cfg(all(test, windows))]
mod windows_immutable_object_handle_tests {
    use super::*;

    #[test]
    fn denies_writes_and_replacement_while_a_range_handle_is_open() {
        let directory = std::env::temp_dir().join(format!(
            "rusthouse-immutable-range-win-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("part.bin");
        fs::write(&path, [42u8; 128]).unwrap();

        let (file, baseline) = open_verified_object(&path).unwrap();
        assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(fs::remove_file(&path).is_err());
        let mut bytes = [0u8; 16];
        let mut reading = file.try_clone().unwrap();
        read_at_into(&mut reading, 32, &mut bytes).unwrap();
        assert_eq!(bytes, [42u8; 16]);
        verify_object_after_read(&path, &file, &baseline).unwrap();
        drop(reading);
        drop(file);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}

/// One disjoint physical read covering one or more logical range requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoalescedReadSpan {
    pub offset: u64,
    pub length: usize,
}

/// Merge overlapping and adjacent logical reads into minimal physical spans.
///
/// The original requested-byte sum, not just the union size, must fit the
/// existing 8 MiB read budget. All bounds are checked before allocating data.
pub fn plan_coalesced_object_ranges(
    ranges: &[(u64, usize)],
    file_length: u64,
    max_total_bytes: usize,
) -> io::Result<Vec<CoalescedReadSpan>> {
    if ranges.len() > MAX_RANGES {
        return Err(io::Error::other("too many object ranges"));
    }
    let mut requested = 0usize;
    let mut ordered = Vec::with_capacity(ranges.len());
    for &(offset, length) in ranges {
        requested = requested
            .checked_add(length)
            .ok_or_else(|| io::Error::other("object range byte overflow"))?;
        if requested > max_total_bytes || requested > ABSOLUTE_MAX_RANGE {
            return Err(io::Error::other("object ranges exceed byte limit"));
        }
        validate_range(offset, length, file_length)?;
        if length != 0 {
            ordered.push(CoalescedReadSpan { offset, length });
        }
    }
    ordered.sort_unstable_by_key(|span| span.offset);
    let mut merged = Vec::<CoalescedReadSpan>::with_capacity(ordered.len());
    for span in ordered {
        if let Some(last) = merged.last_mut() {
            let end = last.offset + last.length as u64;
            if span.offset <= end {
                let new_end = end.max(span.offset + span.length as u64);
                last.length = (new_end - last.offset) as usize;
                continue;
            }
        }
        merged.push(span);
    }
    Ok(merged)
}

/// Coalesce overlapping requests into one verified read per physical span.
///
/// Callers choosing this API trade up to 8 MiB of temporary memory for fewer
/// positioned file reads. Destination buffers can be partially modified on
/// an I/O or identity error and must then be discarded.
pub fn read_coalesced_object_ranges_into(
    path: &Path,
    ranges: &mut [(u64, &mut [u8])],
    max_total_bytes: usize,
) -> io::Result<()> {
    if ranges.len() > MAX_RANGES {
        return Err(io::Error::other("too many object ranges"));
    }
    let (mut file, baseline) = open_verified_object(path)?;
    let requests = ranges
        .iter()
        .map(|(offset, bytes)| (*offset, bytes.len()))
        .collect::<Vec<_>>();
    let spans = plan_coalesced_object_ranges(&requests, baseline.len(), max_total_bytes)?;
    for span in spans {
        let mut bytes = vec![0u8; span.length];
        read_at_into(&mut file, span.offset, &mut bytes)?;
        let end = span.offset + span.length as u64;
        for (offset, destination) in ranges.iter_mut() {
            if destination.is_empty() {
                continue;
            }
            let request_end = *offset + destination.len() as u64;
            if *offset >= span.offset && request_end <= end {
                let start = (*offset - span.offset) as usize;
                let length = destination.len();
                destination.copy_from_slice(&bytes[start..start + length]);
            }
        }
    }
    verify_object_after_read(path, &file, &baseline)
}

#[cfg(test)]
#[path = "distributed_range_read_tests/coalesced_object_read_tests.rs"]
mod coalesced_object_read_tests;

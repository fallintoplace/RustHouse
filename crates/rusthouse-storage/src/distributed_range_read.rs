//! Safe seek-and-read support for a future replica object service.
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const ABSOLUTE_MAX_RANGE: usize = 8 * 1024 * 1024;

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

fn open_verified_object(path: &Path) -> io::Result<(File, u64)> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(io::Error::other("object is not a regular file"));
    }
    let file = File::open(path)?;
    let opened = file.metadata()?;
    if !opened.file_type().is_file() || opened.len() != meta.len() {
        return Err(io::Error::other("object changed before range read"));
    }
    #[cfg(unix)]
    if !same_object_identity(&meta, &opened) {
        return Err(io::Error::other("object inode changed before range read"));
    }
    Ok((file, opened.len()))
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
    let (mut file, file_length) = open_verified_object(path)?;
    validate_range(offset, length, file_length)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Read several exact byte ranges using one file handle and a single memory budget.
/// All range bounds are checked before any bytes are returned.
pub fn read_object_ranges(
    path: &Path,
    ranges: &[(u64, usize)],
    max_total_bytes: usize,
) -> io::Result<Vec<Vec<u8>>> {
    if ranges.len() > 1024 {
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
    let (mut file, file_length) = open_verified_object(path)?;
    for &(offset, length) in ranges {
        validate_range(offset, length, file_length)?;
    }
    if let [(offset, length)] = ranges {
        // A single range needs neither interval sorting nor a second copy.
        let mut data = vec![0u8; *length];
        file.seek(SeekFrom::Start(*offset))?;
        file.read_exact(&mut data)?;
        return Ok(vec![data]);
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
        let mut chunk = vec![0u8; bytes];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;
        for &(index, offset, item_end) in &ordered[cursor..tail] {
            let from = (offset - start) as usize;
            let to = (item_end - start) as usize;
            output[index] = chunk[from..to].to_vec();
        }
        cursor = tail;
    }
    Ok(output)
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

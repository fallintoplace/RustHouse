//! Safe seek-and-read support for a future replica object service.
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const ABSOLUTE_MAX_RANGE: usize = 8 * 1024 * 1024;

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
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(io::Error::other("object is not a regular file"));
    }
    let end = offset
        .checked_add(length as u64)
        .ok_or_else(|| io::Error::other("object range overflow"))?;
    if end > meta.len() {
        return Err(io::Error::other("object range exceeds file length"));
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exact_intervals_and_rejects_bad_ranges() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-range-{}",
            std::process::id()
        ));
        fs::write(&path, b"0123456789").unwrap();
        assert_eq!(read_object_range(&path, 3, 4, 8).unwrap(), b"3456");
        assert_eq!(read_object_range(&path, 10, 0, 8).unwrap(), b"");
        assert!(read_object_range(&path, 9, 2, 8).is_err());
        assert!(read_object_range(&path, u64::MAX, 2, 8).is_err());
        assert!(read_object_range(&path, 0, 9, 8).is_err());
        fs::remove_file(path).unwrap();
    }
}
//! Safe seek-and-read support for a future replica object service.
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const ABSOLUTE_MAX_RANGE: usize = 8 * 1024 * 1024;

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
    let end = offset
        .checked_add(length as u64)
        .ok_or_else(|| io::Error::other("object range overflow"))?;
    if end > file_length {
        return Err(io::Error::other("object range exceeds file length"));
    }
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
    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_range_sources() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-range-link-{}", std::process::id()
        ));
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
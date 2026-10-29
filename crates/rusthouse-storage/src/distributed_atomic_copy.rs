//! Copy a frozen local file into replica storage without overwriting an object.
//! This is a filesystem transport, not an authenticated network protocol.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
fn same_source_identity(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    before.dev() == after.dev() && before.ino() == after.ino()
}

fn stream_bounded_copy(
    input: &mut File,
    output: &mut File,
    expected_bytes: u64,
    max_bytes: u64,
) -> io::Result<u64> {
    let mut total = 0u64;
    // Small files avoid allocating a large buffer; large copies reduce syscall count.
    let buffer_bytes = expected_bytes.clamp(1, 256 * 1024) as usize;
    let mut buffer = vec![0u8; buffer_bytes];
    loop {
        let available = max_bytes
            .saturating_sub(total)
            .saturating_add(1)
            .min(buffer.len() as u64) as usize;
        let len = input.read(&mut buffer[..available])?;
        if len == 0 {
            break;
        }
        total = total
            .checked_add(len as u64)
            .ok_or_else(|| io::Error::other("replica size overflow"))?;
        if total > max_bytes {
            return Err(io::Error::other("replica file exceeds byte cap"));
        }
        output.write_all(&buffer[..len])?;
    }
    if total != expected_bytes {
        return Err(io::Error::other("source changed during immutable copy"));
    }
    output.sync_all()?;
    Ok(total)
}

/// Stage, sync, and hard-link an immutable destination file.
/// Both paths must be on a trusted local filesystem. Existing objects are never replaced.
pub fn copy_immutable_file(source: &Path, destination: &Path, max_bytes: u64) -> io::Result<u64> {
    let source_meta = fs::symlink_metadata(source)?;
    if !source_meta.file_type().is_file() || source_meta.len() > max_bytes {
        return Err(io::Error::other("source is not a bounded regular file"));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("destination has no parent directory"))?;
    if !fs::symlink_metadata(parent)?.file_type().is_dir() {
        return Err(io::Error::other(
            "destination parent is not a real directory",
        ));
    }
    let staging = parent.join(format!(
        ".rusthouse-replica-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut input = File::open(source)?;
    let opened = input.metadata()?;
    if !opened.file_type().is_file() || opened.len() != source_meta.len() {
        return Err(io::Error::other("source changed before immutable copy"));
    }
    #[cfg(unix)]
    if !same_source_identity(&source_meta, &opened) {
        return Err(io::Error::other("source inode changed before immutable copy"));
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?;
    let copied = stream_bounded_copy(&mut input, &mut output, source_meta.len(), max_bytes);
    drop(output);
    // hard_link fails if the destination already exists, unlike an overwriting rename.
    let published = copied.and_then(|total| {
        fs::hard_link(&staging, destination)?;
        Ok(total)
    });
    let cleanup = fs::remove_file(&staging);
    let total = match published {
        Ok(total) => {
            cleanup?;
            total
        }
        Err(error) => {
            let _ = cleanup;
            return Err(error);
        }
    };
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(total)
}

/// Copy a frozen file and publish it only when the staged bytes match CRC32C.
/// CRC32C detects accidental corruption, not malicious content modification.
pub fn copy_immutable_file_checked(
    source: &Path,
    destination: &Path,
    max_bytes: u64,
    expected_crc32c: u32,
) -> io::Result<u64> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("destination has no parent directory"))?;
    let staging = parent.join(format!(
        ".rusthouse-checked-copy-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
    ));
    let copied = copy_immutable_file(source, &staging, max_bytes)?;
    let verified = (|| -> io::Result<()> {
        let mut file = File::open(&staging)?;
        let (size, crc32c) = crate::distributed_crc32c::crc32c_reader(&mut file, max_bytes)?;
        if size != copied || crc32c != expected_crc32c {
            return Err(io::Error::other("replica object checksum mismatch"));
        }
        fs::hard_link(&staging, destination)?;
        Ok(())
    })();
    let cleanup = fs::remove_file(&staging);
    verified?;
    cleanup?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(copied)
}

#[cfg(all(test, unix))]
mod copy_identity_tests {
    use super::*;

    #[test]
    fn identical_sizes_do_not_make_different_inodes_equivalent() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-copy-source-id-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first");
        let second = root.join("second");
        let link = root.join("link");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        fs::hard_link(&first, &link).unwrap();
        assert!(same_source_identity(&fs::metadata(&first).unwrap(), &fs::metadata(&link).unwrap()));
        assert!(!same_source_identity(&fs::metadata(&first).unwrap(), &fs::metadata(&second).unwrap()));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Seek;

    #[test]
    fn copies_once_and_never_replaces_existing_data() {
        let dir =
            std::env::temp_dir().join(format!("rusthouse-atomic-copy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let src = dir.join("source");
        let dst = dir.join("replica");
        fs::write(&src, b"immutable data").unwrap();
        assert_eq!(copy_immutable_file(&src, &dst, 100).unwrap(), 14);
        assert_eq!(fs::read(&dst).unwrap(), b"immutable data");
        assert!(copy_immutable_file(&src, &dst, 100).is_err());
        assert!(copy_immutable_file(&src, &dir.join("small"), 1).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn oversized_source_reads_only_one_byte_past_cap() {
        let dir =
            std::env::temp_dir().join(format!("rusthouse-bounded-copy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source");
        let target = dir.join("target");
        fs::write(&source, vec![1u8; 64 * 1024]).unwrap();
        let mut input = File::open(&source).unwrap();
        let mut output = File::create(&target).unwrap();
        assert!(stream_bounded_copy(&mut input, &mut output, 64 * 1024, 0).is_err());
        assert_eq!(input.stream_position().unwrap(), 1);
        assert_eq!(output.metadata().unwrap().len(), 0);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod checked_copy_tests {
    use super::*;

    #[test]
    fn checks_staged_bytes_before_publishing() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-checked-copy-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source");
        let target = dir.join("target");
        fs::write(&source, b"verified").unwrap();
        assert!(copy_immutable_file_checked(&source, &target, 100, 0).is_err());
        assert!(!target.exists());
        let checksum = crate::distributed_crc32c::crc32c(b"verified");
        assert_eq!(copy_immutable_file_checked(&source, &target, 100, checksum).unwrap(), 8);
        assert_eq!(fs::read(&target).unwrap(), b"verified");
        assert!(copy_immutable_file_checked(&source, &target, 100, checksum).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

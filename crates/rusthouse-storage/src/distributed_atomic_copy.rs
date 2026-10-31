//! Copy a frozen local file into replica storage without overwriting an object.
//! This is a filesystem transport, not an authenticated network protocol.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
fn same_source_identity(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    before.dev() == after.dev() && before.ino() == after.ino()
}

fn source_unchanged(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    if !after.file_type().is_file()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return same_source_identity(before, after)
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec();
    }
    #[cfg(not(unix))]
    true
}


fn staging_path(parent: &Path, purpose: &str) -> PathBuf {
    parent.join(format!(
        ".rusthouse-{purpose}-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn create_private_replica_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}


// A crashed copy may leave its private staging file behind. A later process
// can reuse the same PID, so never let one orphan block every future copy.
fn create_replica_staging_with(
    mut candidate: impl FnMut() -> PathBuf,
) -> io::Result<(PathBuf, File)> {
    const MAX_COLLISION_RETRIES: usize = 32;
    for _ in 0..MAX_COLLISION_RETRIES {
        let path = candidate();
        match create_private_replica_file(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "replica staging namespace exhausted",
    ))
}

fn create_replica_staging_file(parent: &Path) -> io::Result<(PathBuf, File)> {
    create_replica_staging_with(|| staging_path(parent, "replica"))
}

// A retry must not stream a large immutable object only to discover
// an existing destination when it attempts the final hard link.
fn require_new_destination(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(io::Error::new(io::ErrorKind::AlreadyExists,
            "immutable replica destination already exists")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn stream_bounded_copy(
    input: &mut impl Read,
    output: &mut File,
    expected_bytes: u64,
    max_bytes: u64,
    mut checksum: Option<&mut crate::distributed_crc32c::Crc32cState>,
) -> io::Result<u64> {
    let mut total = 0u64;
    // Most replica metadata files fit in 4 KiB. Use stack storage for
    // these copies and retain the 256 KiB heap buffer for large files.
    let buffer_bytes = expected_bytes.clamp(1, 256 * 1024) as usize;
    let mut inline = [0u8; 4096];
    let mut allocated = Vec::new();
    let buffer: &mut [u8] = if buffer_bytes <= inline.len() {
        &mut inline[..buffer_bytes]
    } else {
        allocated.resize(buffer_bytes, 0);
        &mut allocated
    };
    loop {
        let available = max_bytes
            .saturating_sub(total)
            .saturating_add(1)
            .min(buffer.len() as u64) as usize;
        let len = match input.read(&mut buffer[..available]) {
            Ok(len) => len,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
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
        if let Some(state) = checksum.as_mut() {
            state.append(&buffer[..len]);
        }
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
    copy_immutable_file_impl(source, destination, max_bytes, None)
}

fn copy_immutable_file_impl(
    source: &Path,
    destination: &Path,
    max_bytes: u64,
    expected_crc32c: Option<u32>,
) -> io::Result<u64> {
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
    require_new_destination(destination)?;
    let mut input = File::open(source)?;
    let opened = input.metadata()?;
    // An in-place same-length rewrite between symlink_metadata and open
    // must not be treated as the immutable source that was verified.
    if !source_unchanged(&source_meta, &opened) {
        return Err(io::Error::other("source changed before immutable copy"));
    }
    let (staging, mut output) = create_replica_staging_file(parent)?;
    let mut checksum = expected_crc32c.map(|_| crate::distributed_crc32c::Crc32cState::new());
    let copied = stream_bounded_copy(
        &mut input, &mut output, source_meta.len(), max_bytes, checksum.as_mut(),
    );
    drop(output);
    // hard_link fails if the destination already exists, unlike an overwriting rename.
    let published = copied.and_then(|total| {
        if let Some(expected) = expected_crc32c {
            if checksum.as_ref().is_none_or(|state| state.checksum() != expected) {
                return Err(io::Error::other("replica object checksum mismatch"));
            }
        }
        // Recheck both the opened inode and the source path after the stream.
        // A mutation or replacement at the same length must not be published.
        let final_opened = input.metadata()?;
        let final_path = fs::symlink_metadata(source)?;
        if !source_unchanged(&source_meta, &final_opened)
            || !source_unchanged(&source_meta, &final_path)
        {
            return Err(io::Error::other("source changed during immutable copy"));
        }
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
    // Compute CRC32C while the copy is streamed. Publish only after the
    // checksum and frozen-source identity both pass, without a second read.
    copy_immutable_file_impl(source, destination, max_bytes, Some(expected_crc32c))
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
        let baseline = fs::metadata(&first).unwrap();
        assert!(source_unchanged(&baseline, &baseline));
        assert!(!source_unchanged(&baseline, &fs::metadata(&second).unwrap()));
        fs::write(&first, b"longer").unwrap();
        assert!(!source_unchanged(&baseline, &fs::metadata(&first).unwrap()));
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
        assert!(stream_bounded_copy(&mut input, &mut output, 64 * 1024, 0, None).is_err());
        assert_eq!(input.stream_position().unwrap(), 1);
        assert_eq!(output.metadata().unwrap().len(), 0);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod checked_copy_tests {
    use super::*;

    #[test]
    fn checked_copy_streams_large_inputs_and_rejects_corruption() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-checked-copy-stream-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source");
        let target = dir.join("target");
        let payload = (0..300_000).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        fs::write(&source, &payload).unwrap();
        let expected = crate::distributed_crc32c::crc32c(&payload);
        assert!(copy_immutable_file_checked(
            &source, &target, payload.len() as u64, expected ^ 1,
        ).is_err());
        assert!(!target.exists());
        assert_eq!(
            copy_immutable_file_checked(&source, &target, payload.len() as u64, expected).unwrap(),
            payload.len() as u64,
        );
        assert_eq!(fs::read(&target).unwrap(), payload);
        fs::remove_dir_all(dir).unwrap();
    }

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

#[cfg(test)]
mod interrupted_replica_copy_tests {
    use super::*;
    use std::io::Cursor;

    struct InterruptOnce<'a> {
        input: Cursor<&'a [u8]>,
        interrupted: bool,
    }

    impl Read for InterruptOnce<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.interrupted {
                self.interrupted = false;
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.input.read(output)
        }
    }

    #[test]
    fn retries_interrupted_source_reads_without_publishing_partial_data() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-interrupted-replica-copy-{}", std::process::id()
        ));
        let mut output = File::create(&path).unwrap();
        let mut input = InterruptOnce {
            input: Cursor::new(b"frozen bytes".as_slice()),
            interrupted: true,
        };
        assert_eq!(
            stream_bounded_copy(&mut input, &mut output, 12, 12, None).unwrap(),
            12
        );
        drop(output);
        assert_eq!(fs::read(&path).unwrap(), b"frozen bytes");
        fs::remove_file(path).unwrap();
    }
}

#[cfg(all(test, unix))]
mod private_replica_object_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn new_replica_files_are_private_before_publication() {
        let file = std::env::temp_dir().join(format!(
            "rusthouse-private-replica-{}", std::process::id()
        ));
        let _ = fs::remove_file(&file);
        let output = create_private_replica_file(&file).unwrap();
        assert_eq!(output.metadata().unwrap().permissions().mode() & 0o077, 0);
        drop(output);
        fs::remove_file(file).unwrap();
    }
}

#[cfg(test)]
mod existing_destination_preflight_tests {
    use super::*;

    #[test]
    fn retries_do_not_create_staging_files_for_occupied_destinations() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-copy-existing-{}-{}", std::process::id(), line!()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source");
        let destination = dir.join("destination");
        fs::write(&source, vec![0x42u8; 1024 * 1024]).unwrap();
        fs::write(&destination, b"old immutable data").unwrap();
        let before = fs::read_dir(&dir).unwrap().count();
        assert_eq!(
            copy_immutable_file(&source, &destination, u64::MAX).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let checksum = crate::distributed_crc32c::crc32c(&fs::read(&source).unwrap());
        assert_eq!(
            copy_immutable_file_checked(&source, &destination, u64::MAX, checksum)
                .unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), before);
        assert_eq!(fs::read(&destination).unwrap(), b"old immutable data");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_targets_are_not_treated_as_free_destinations() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "rusthouse-copy-symlink-destination-{}-{}", std::process::id(), line!()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let source = root.join("source");
        fs::write(&source, b"content").unwrap();
        let link = root.join("destination");
        symlink("missing", &link).unwrap();
        assert_eq!(
            copy_immutable_file(&source, &link, 1024).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod orphaned_replica_staging_tests {
    use super::*;

    #[test]
    fn collision_retry_never_overwrites_abandoned_private_bytes() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-stage-retry-{}-{}", std::process::id(), line!()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let orphan = dir.join(".rusthouse-replica-orphan");
        let fresh = dir.join(".rusthouse-replica-fresh");
        fs::write(&orphan, b"uncommitted old transfer").unwrap();
        let mut candidates = [orphan.clone(), fresh.clone()].into_iter();
        let (staging, mut file) =
            create_replica_staging_with(|| candidates.next().expect("bounded retries")).unwrap();
        assert_eq!(staging, fresh);
        file.write_all(b"new transfer").unwrap();
        drop(file);
        assert_eq!(fs::read(&orphan).unwrap(), b"uncommitted old transfer");
        assert_eq!(fs::read(&fresh).unwrap(), b"new transfer");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cannot_overwrite_even_when_all_staging_names_collide() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-stage-exhaustion-{}-{}", std::process::id(), line!()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let stale = dir.join("stale");
        fs::write(&stale, b"persistent").unwrap();
        let err = create_replica_staging_with(|| stale.clone()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&stale).unwrap(), b"persistent");
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod source_open_metadata_tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn same_length_source_mutations_break_snapshot_identity() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-copy-open-check-{}-{:?}",
            std::process::id(), std::thread::current().id()
        ));
        fs::write(&root, b"original").unwrap();
        let before = fs::symlink_metadata(&root).unwrap();
        let file = OpenOptions::new().write(true).open(&root).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)
        )).unwrap();
        let after = file.metadata().unwrap();
        assert_eq!(before.len(), after.len());
        assert!(!source_unchanged(&before, &after));
        fs::remove_file(root).unwrap();
    }
}

#[cfg(test)]
mod small_replica_copy_stack_buffer_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn stack_and_heap_copy_boundaries_preserve_bytes_and_streamed_crc() {
        let directory = std::env::temp_dir().join(format!(
            "rusthouse-small-copy-{}-{:?}",
            std::process::id(), std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        for size in [0usize, 1, 127, 4095, 4096, 4097, 262_144, 262_145] {
            let payload = (0..size).map(|index| (index % 251) as u8).collect::<Vec<_>>();
            let mut input = Cursor::new(payload.as_slice());
            let target = directory.join(format!("target-{size}"));
            let mut output = File::create(&target).unwrap();
            let mut state = crate::distributed_crc32c::Crc32cState::new();
            assert_eq!(
                stream_bounded_copy(
                    &mut input, &mut output, size as u64, size as u64,
                    Some(&mut state),
                ).unwrap(),
                size as u64,
            );
            drop(output);
            assert_eq!(fs::read(&target).unwrap(), payload);
            assert_eq!(state.checksum(), crate::distributed_crc32c::crc32c(&payload));
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn small_copy_still_rejects_unexpected_extra_bytes() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-small-copy-extra-{}-{:?}",
            std::process::id(), std::thread::current().id()
        ));
        let mut input = Cursor::new(b"12345".as_slice());
        let mut output = File::create(&path).unwrap();
        assert!(stream_bounded_copy(&mut input, &mut output, 4, 4, None).is_err());
        assert_eq!(input.position(), 5);
        assert_eq!(output.metadata().unwrap().len(), 4);
        drop(output);
        fs::remove_file(path).unwrap();
    }
}

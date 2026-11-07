use super::*;

pub(super) fn has_trailing_byte(reader: &mut impl Read) -> io::Result<bool> {
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(count) => return Ok(count != 0),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

// A same-length in-place rewrite can preserve mtime while changing ctime.
// Check both timestamps on Unix before trusting a checksum of an immutable file.
pub(super) fn stored_file_state_unchanged(before: &fs::Metadata, after: &fs::Metadata) -> io::Result<bool> {
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(before.ctime() == after.ctime() && before.ctime_nsec() == after.ctime_nsec())
    }
    #[cfg(not(unix))]
    {
        Ok(true)
    }
}

// A stored-byte checksum is only meaningful if the source cannot change
// between its verified metadata and the last hashed byte.
#[cfg(windows)]
pub(super) fn open_stored_hash_file(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
}

#[cfg(not(windows))]
pub(super) fn open_stored_hash_file(path: &Path) -> io::Result<fs::File> {
    fs::File::open(path)
}

pub(super) fn hash_stored_file(path: &Path, expected_size: u64) -> Result<u128, PartError> {
    const BLOCK_BYTES: usize = 2048;
    let mut file = open_stored_hash_file(path).map_err(|source| io_error(path, source))?;
    #[cfg(unix)]
    ensure_opened_file_identity(path, &file)?;
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    let mut hash = U128::new(0, 0);
    let mut remaining = expected_size;
    let mut buffer = [0u8; BLOCK_BYTES];
    while remaining > 0 {
        let length = remaining.min(BLOCK_BYTES as u64) as usize;
        file.read_exact(&mut buffer[..length])
            .map_err(|source| io_error(path, source))?;
        hash = cityhash128_with_seed(&buffer[..length], hash);
        remaining -= length as u64;
    }
    // Catch files extended after the size check, as well as changed data.
    if has_trailing_byte(&mut file).map_err(|source| io_error(path, source))? {
        return Err(invalid(path, "file changed during hash verification"));
    }
    #[cfg(unix)]
    ensure_opened_file_identity(path, &file)?;
    let finished = file.metadata().map_err(|source| io_error(path, source))?;
    if !stored_file_state_unchanged(&opened, &finished).map_err(|source| io_error(path, source))? {
        return Err(invalid(path, "file changed during hash verification"));
    }
    Ok(u128::from(hash))
}

pub(super) fn verify_stored_hash(
    path: &Path,
    expected_size: u64,
    expected_hash: u128,
) -> Result<(), PartError> {
    if hash_stored_file(path, expected_size)? != expected_hash {
        return Err(invalid(path, "stored file hash differs from checksums.txt"));
    }
    Ok(())
}

// The compressed input is read through an opened, validated file descriptor.
// The path and inode must still describe the same immutable bytes after decode.
pub(super) fn verify_compressed_input_unchanged(
    path: &Path,
    initial: &fs::Metadata,
    finished: &fs::Metadata,
) -> Result<(), PartError> {
    let current = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !current.file_type().is_file()
        || !stored_file_state_unchanged(initial, finished)
            .map_err(|source| io_error(path, source))?
        || !stored_file_state_unchanged(initial, &current)
            .map_err(|source| io_error(path, source))?
    {
        return Err(invalid(
            path,
            "compressed stream changed during hash verification",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if initial.dev() != current.dev()
            || initial.ino() != current.ino()
            || initial.dev() != finished.dev()
            || initial.ino() != finished.ino()
        {
            return Err(invalid(
                path,
                "compressed stream replaced during hash verification",
            ));
        }
    }
    Ok(())
}

/// Hash the logical byte stream, not the individual compressed block boundaries.
/// ClickHouse's HashingWriteBuffer uses a continuous 2048-byte chunk sequence.
pub(super) fn verify_uncompressed_hash(path: &Path, expected: &UncompressedChecksum) -> Result<(), PartError> {
    const BLOCK_BYTES: usize = 2048;
    let mut reader = CompressedBlockReader::open(path)?;
    let initial = reader
        .opened_metadata()
        .map_err(|source| io_error(path, source))?;
    let mut hash = U128::new(0, 0);
    let mut chunk = [0u8; BLOCK_BYTES];
    let mut filled = 0usize;
    let mut total_bytes = 0u64;
    while let Some(block) = reader.next_decoded() {
        let data = block?.data;
        total_bytes = total_bytes
            .checked_add(data.len() as u64)
            .ok_or_else(|| invalid(path, "uncompressed file size overflows u64"))?;
        if total_bytes > expected.size {
            return Err(invalid(
                path,
                "uncompressed file size differs from checksums.txt",
            ));
        }
        let mut remaining = data.as_slice();
        while !remaining.is_empty() {
            let take = (BLOCK_BYTES - filled).min(remaining.len());
            chunk[filled..filled + take].copy_from_slice(&remaining[..take]);
            filled += take;
            remaining = &remaining[take..];
            if filled == BLOCK_BYTES {
                hash = cityhash128_with_seed(&chunk, hash);
                filled = 0;
            }
        }
    }
    if total_bytes != expected.size {
        return Err(invalid(
            path,
            "uncompressed file size differs from checksums.txt",
        ));
    }
    if filled > 0 {
        hash = cityhash128_with_seed(&chunk[..filled], hash);
    }
    if u128::from(hash) != expected.hash {
        return Err(invalid(
            path,
            "uncompressed file hash differs from checksums.txt",
        ));
    }
    let finished = reader
        .opened_metadata()
        .map_err(|source| io_error(path, source))?;
    verify_compressed_input_unchanged(path, &initial, &finished)?;
    Ok(())
}

/// Detect inode replacement between path validation, open, and hash completion.
#[cfg(unix)]
pub(super) fn ensure_opened_file_identity(path: &Path, file: &fs::File) -> Result<(), PartError> {
    use std::os::unix::fs::MetadataExt;

    let named = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    if !named.file_type().is_file()
        || !opened.file_type().is_file()
        || named.dev() != opened.dev()
        || named.ino() != opened.ino()
    {
        return Err(invalid(
            path,
            "listed file was replaced during hash verification",
        ));
    }
    Ok(())
}

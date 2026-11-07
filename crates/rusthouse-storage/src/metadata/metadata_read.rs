use super::*;

pub(super) fn ensure_metadata_size(path: &Path, size: u64, max_bytes: u64) -> Result<(), PartError> {
    if size > max_bytes {
        return Err(invalid_metadata(path, "metadata exceeds size limit"));
    }
    Ok(())
}

// A stable length and mtime are not sufficient for frozen part metadata.
// Include Unix inode and change time to catch same-sized replacements.
pub(super) fn metadata_unchanged(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    if !after.file_type().is_file()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }
    #[cfg(not(unix))]
    true
}

pub(super) fn ensure_inspection_generation(path: &Path, before: &fs::Metadata) -> Result<(), PartError> {
    let after = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata_unchanged(before, &after) {
        return Err(invalid_metadata(
            path,
            "part metadata changed during inspection",
        ));
    }
    Ok(())
}

pub(super) fn read_bounded_metadata_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, PartError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_file() {
        return Err(invalid_metadata(path, "metadata must be a regular file"));
    }
    ensure_metadata_size(path, metadata.len(), max_bytes)?;
    let mut file = fs::File::open(path).map_err(|source| io_error(path, source))?;
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    if !metadata_unchanged(&metadata, &opened) || opened.len() > max_bytes {
        return Err(invalid_metadata(path, "metadata changed before read"));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    (&mut file)
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    ensure_metadata_size(path, bytes.len() as u64, max_bytes)?;
    ensure_exact_metadata_length(path, opened.len(), bytes.len())?;
    let finished = file.metadata().map_err(|source| io_error(path, source))?;
    let current = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata_unchanged(&opened, &finished) || !metadata_unchanged(&opened, &current) {
        return Err(invalid_metadata(path, "metadata changed during read"));
    }
    Ok(bytes)
}

pub(super) fn read_metadata(path: &Path, max_bytes: u64) -> Result<String, PartError> {
    String::from_utf8(read_bounded_metadata_file(path, max_bytes)?)
        .map_err(|source| io_error(path, io::Error::new(io::ErrorKind::InvalidData, source)))
}

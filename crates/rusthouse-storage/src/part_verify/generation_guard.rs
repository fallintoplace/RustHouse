use super::*;

/// A verified wide part must describe one immutable generation across both
/// mark validation and full block decoding. Individual open-time checks do
/// not detect a valid stream being replaced between those two passes.
pub(super) fn same_verification_generation(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    if !before.file_type().is_file()
        || !after.file_type().is_file()
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
    {
        true
    }
}

/// Pin the complete set of files before the slow validation scan. Checking
/// one stream immediately after reading it is not sufficient: that stream
/// could be replaced while a later stream is still being decoded.
pub(super) fn pin_verification_files(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<(PathBuf, fs::Metadata)>, PartError> {
    let mut pinned = Vec::new();
    for path in paths {
        let metadata = fs::symlink_metadata(&path).map_err(|source| io_error(&path, source))?;
        if !metadata.file_type().is_file() {
            return Err(invalid(
                &path,
                "wide verification source is not a regular file",
            ));
        }
        pinned.push((path, metadata));
    }
    Ok(pinned)
}

pub(super) fn verify_pinned_files(pinned: &[(PathBuf, fs::Metadata)]) -> Result<(), PartError> {
    for (path, original) in pinned {
        let current = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
        if !same_verification_generation(original, &current) {
            return Err(invalid(path, "wide part changed during verification"));
        }
    }
    Ok(())
}

/// Reject a part directory whose data streams or mark layout changed
/// while verification was scanning earlier streams. File identity checks
/// alone cannot detect newly added files that were never opened.
pub(super) fn verify_stream_set_unchanged(
    part: &Path,
    initial_data: &BTreeSet<PathBuf>,
    initial_marks: &BTreeMap<PathBuf, PathBuf>,
) -> Result<(), PartError> {
    let (current_data, current_marks) = collect_wide_file_pairs(part)?;
    if &current_data != initial_data || &current_marks != initial_marks {
        return Err(invalid(
            part,
            "wide part stream set changed during verification",
        ));
    }
    Ok(())
}

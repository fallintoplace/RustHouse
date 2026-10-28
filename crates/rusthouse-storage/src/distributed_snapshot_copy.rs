//! Offline copy of a frozen set of immutable wide parts to a new local root.
//! Both source and destination require trusted filesystems with stopped writers.
use crate::{
    PartError, inspect_query_source, query_parts, resolve_active_snapshot, verify_part_file_hashes,
    verify_wide_part,
};
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_COPY: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotCopy {
    pub parts: usize,
    pub files: usize,
    pub bytes: u64,
}

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Copy a complete active generation into a new directory and validate it
/// before rename. Does not copy catalog schemas or create remote connections.
pub fn copy_frozen_snapshot(
    source: &Path,
    destination: &Path,
    max_parts: usize,
    max_bytes: u64,
) -> Result<SnapshotCopy, PartError> {
    let active = resolve_active_snapshot(source)?;
    let parts = query_parts(&active)?;
    if parts.len() > max_parts || max_parts > 16_384 {
        return Err(PartError::InvalidScan(
            "snapshot copy exceeds part limit".into(),
        ));
    }
    inspect_query_source(&active)?;
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(PartError::InvalidScan("destination already exists".into())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(destination, error)),
    }
    let parent = destination
        .parent()
        .ok_or_else(|| PartError::InvalidScan("destination has no parent".into()))?;
    if !fs::symlink_metadata(parent)
        .map_err(|error| io_error(parent, error))?
        .file_type()
        .is_dir()
    {
        return Err(PartError::InvalidScan(
            "destination parent is not a directory".into(),
        ));
    }
    let staging = parent.join(format!(
        ".rusthouse-snapshot-copy-{}-{}",
        std::process::id(),
        NEXT_COPY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&staging).map_err(|error| io_error(&staging, error))?;
    let copied = (|| -> Result<SnapshotCopy, PartError> {
        let mut report = SnapshotCopy {
            parts: 0,
            files: 0,
            bytes: 0,
        };
        for part in &parts {
            verify_part_file_hashes(part)?;
            let name = part
                .file_name()
                .ok_or_else(|| PartError::InvalidScan("part has no name".into()))?;
            let copy = staging.join(name);
            fs::create_dir(&copy).map_err(|error| io_error(&copy, error))?;
            for entry in fs::read_dir(part).map_err(|error| io_error(part, error))? {
                let entry = entry.map_err(|error| io_error(part, error))?;
                let source_file = entry.path();
                if !entry
                    .file_type()
                    .map_err(|error| io_error(&source_file, error))?
                    .is_file()
                {
                    return Err(PartError::InvalidScan(
                        "part contains non-file entries".into(),
                    ));
                }
                let destination_file = copy.join(entry.file_name());
                let bytes = entry
                    .metadata()
                    .map_err(|error| io_error(&source_file, error))?
                    .len();
                report.bytes = report
                    .bytes
                    .checked_add(bytes)
                    .ok_or_else(|| PartError::InvalidScan("copy size overflow".into()))?;
                if report.bytes > max_bytes {
                    return Err(PartError::InvalidScan("copy exceeds byte cap".into()));
                }
                let copied_bytes = fs::copy(&source_file, &destination_file)
                    .map_err(|error| io_error(&destination_file, error))?;
                if copied_bytes != bytes {
                    return Err(PartError::InvalidScan(
                        "source size changed during snapshot copy".into(),
                    ));
                }
                File::open(&destination_file)
                    .and_then(|file| file.sync_all())
                    .map_err(|error| io_error(&destination_file, error))?;
                report.files += 1;
            }
            verify_part_file_hashes(&copy)?;
            verify_wide_part(&copy)?;
            File::open(&copy)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| io_error(&copy, error))?;
            report.parts += 1;
        }
        inspect_query_source(&staging)?;
        File::open(&staging)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| io_error(&staging, error))?;
        Ok(report)
    })();
    let report = match copied {
        Ok(report) => report,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if destination.exists() {
        let _ = fs::remove_dir_all(&staging);
        return Err(PartError::InvalidScan(
            "destination appeared during copy".into(),
        ));
    }
    if let Err(error) = fs::rename(&staging, destination) {
        let _ = fs::remove_dir_all(&staging);
        return Err(io_error(destination, error));
    }
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(parent, error))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn copies_verified_snapshot_and_rejects_overwrites() {
        let base =
            std::env::temp_dir().join(format!("rusthouse-snapshot-copy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let source = base.join("source");
        let target = base.join("replica");
        fs::create_dir_all(&source).unwrap();
        write_wide_part(
            &source.join("all_0_0_0"),
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("12".into())]],
        )
        .unwrap();
        let report = copy_frozen_snapshot(&source, &target, 1, 1_000_000).unwrap();
        assert_eq!(report.parts, 1);
        assert!(report.files > 0);
        assert_eq!(inspect_query_source(&target).unwrap().rows, 1);
        assert!(copy_frozen_snapshot(&source, &target, 1, 1_000_000).is_err());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn corrupted_snapshot_never_publishes_destination() {
        let root =
            std::env::temp_dir().join(format!("rusthouse-copy-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        let part = source.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("12".into())]],
        )
        .unwrap();
        fs::write(part.join("count.txt"), "100\n").unwrap();
        assert!(copy_frozen_snapshot(&source, &target, 1, 1_000_000).is_err());
        assert!(!target.exists());
        fs::remove_dir_all(root).unwrap();
    }
}

/// Check a frozen snapshot and report exact copy requirements without writing files.
pub fn preview_frozen_snapshot(
    source: &Path,
    max_parts: usize,
    max_files: usize,
    max_bytes: u64,
) -> Result<SnapshotCopy, PartError> {
    let active = resolve_active_snapshot(source)?;
    let parts = query_parts(&active)?;
    if parts.len() > max_parts || max_parts > 16_384 {
        return Err(PartError::InvalidScan("snapshot preview exceeds part limit".into()));
    }
    inspect_query_source(&active)?;
    let files = crate::distributed_inventory::inventory_snapshot(source, max_files, max_bytes)?;
    let bytes = files.iter().try_fold(0u64, |total, file| {
        total.checked_add(file.bytes)
            .ok_or_else(|| PartError::InvalidScan("snapshot preview byte overflow".into()))
    })?;
    for part in &parts {
        verify_part_file_hashes(part)?;
        verify_wide_part(part)?;
    }
    Ok(SnapshotCopy {
        parts: parts.len(),
        files: files.len(),
        bytes,
    })
}

#[cfg(test)]
mod snapshot_preflight_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn predicts_copy_requirements_without_publishing_destination() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-snapshot-preview-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).unwrap();
        write_wide_part(
            &source.join("all_0_0_0"),
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("7".into())]],
        ).unwrap();
        let preview = preview_frozen_snapshot(&source, 1, 32, 1_000_000).unwrap();
        assert_eq!(preview.parts, 1);
        assert!(preview.files > 0);
        assert!(!destination.exists());
        assert_eq!(
            copy_frozen_snapshot(&source, &destination, 1, 1_000_000).unwrap(),
            preview
        );
        assert!(preview_frozen_snapshot(&source, 0, 32, 1_000_000).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

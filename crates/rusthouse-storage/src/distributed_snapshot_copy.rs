//! Offline copy of a frozen set of immutable wide parts to a new local root.
//! Both source and destination require trusted filesystems with stopped writers.
use crate::{
    PartError, inspect_query_source, query_parts, resolve_active_snapshot, verify_part_file_hashes,
    verify_wide_part,
};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
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

fn create_private_staging_directory(path: &Path) -> io::Result<()> {
    let mut directory = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(path)
}

// Crashed copies may leave a stage directory behind. A reused PID must
// not prevent a later, independent snapshot from being copied.
fn create_snapshot_staging_with(
    mut candidate: impl FnMut() -> PathBuf,
) -> io::Result<PathBuf> {
    for _ in 0..32 {
        let path = candidate();
        match create_private_staging_directory(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "snapshot staging namespace exhausted",
    ))
}

fn sync_directory(path: &Path) -> Result<(), PartError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(path, error))
}

fn ensure_destination_absent(destination: &Path) -> Result<(), PartError> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(PartError::InvalidScan("destination already exists".into())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(destination, error)),
    }
}


/// Linux can atomically publish a directory without replacing another
/// writer's destination, even if it appears after the absence check.
#[cfg(target_os = "linux")]
fn publish_snapshot_no_replace(staging: &Path, destination: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renameat2(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }
    const AT_FDCWD: c_int = -100;
    const RENAME_NOREPLACE: c_uint = 1;
    let path = |value: &Path| {
        CString::new(value.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in snapshot path"))
    };
    let old = path(staging)?;
    let new = path(destination)?;
    let code = unsafe {
        renameat2(AT_FDCWD, old.as_ptr(), AT_FDCWD, new.as_ptr(), RENAME_NOREPLACE)
    };
    if code == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Other platforms retain the trusted single-writer precondition.
#[cfg(not(target_os = "linux"))]
fn publish_snapshot_no_replace(staging: &Path, destination: &Path) -> io::Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(io::Error::new(io::ErrorKind::AlreadyExists, "destination exists")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(staging, destination),
        Err(error) => Err(error),
    }
}

// One part is an independent unit of immutable copy and verification.
// It writes only to its own staging subdirectory and never publishes the root.
fn copy_snapshot_part(
    part: &Path,
    staging: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<SnapshotCopy, PartError> {
    verify_part_file_hashes(part)?;
    let name = part
        .file_name()
        .ok_or_else(|| PartError::InvalidScan("part has no name".into()))?;
    let copy = staging.join(name);
    fs::create_dir(&copy).map_err(|error| io_error(&copy, error))?;
    let mut report = SnapshotCopy { parts: 1, files: 0, bytes: 0 };
    for entry in fs::read_dir(part).map_err(|error| io_error(part, error))? {
        let entry = entry.map_err(|error| io_error(part, error))?;
        let source_file = entry.path();
        if !entry.file_type().map_err(|error| io_error(&source_file, error))?.is_file() {
            return Err(PartError::InvalidScan("part contains non-file entries".into()));
        }
        let destination_file = copy.join(entry.file_name());
        let bytes = entry.metadata().map_err(|error| io_error(&source_file, error))?.len();
        report.bytes = report.bytes.checked_add(bytes)
            .filter(|&total| total <= max_bytes)
            .ok_or_else(|| PartError::InvalidScan("copy exceeds byte cap".into()))?;
        if report.files >= max_files {
            return Err(PartError::InvalidScan("snapshot copy exceeds file limit".into()));
        }
        let copied_bytes = crate::distributed_atomic_copy::copy_immutable_file(
            &source_file, &destination_file, bytes,
        ).map_err(|error| io_error(&destination_file, error))?;
        if copied_bytes != bytes {
            return Err(PartError::InvalidScan("source size changed during snapshot copy".into()));
        }
        report.files += 1;
    }
    verify_part_file_hashes(&copy)?;
    verify_wide_part(&copy)?;
    sync_directory(&copy)?;
    Ok(report)
}

fn combine_copy_report(
    total: &mut SnapshotCopy,
    part: SnapshotCopy,
    max_files: usize,
    max_bytes: u64,
) -> Result<(), PartError> {
    total.parts += part.parts;
    total.files = total.files.checked_add(part.files)
        .filter(|&files| files <= max_files)
        .ok_or_else(|| PartError::InvalidScan("snapshot copy exceeds file limit".into()))?;
    total.bytes = total.bytes.checked_add(part.bytes)
        .filter(|&bytes| bytes <= max_bytes)
        .ok_or_else(|| PartError::InvalidScan("copy exceeds byte cap".into()))?;
    Ok(())
}

/// Copy independent part directories concurrently on multicore hosts, with
/// the same ordered error and atomic-publication behavior as serial copying.
fn copy_snapshot_parts(
    parts: &[PathBuf],
    staging: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<SnapshotCopy, PartError> {
    const PARALLEL_THRESHOLD: usize = 16;
    const MAX_COPY_WORKERS: usize = 4;
    let workers = std::thread::available_parallelism()
        .map_or(1, |available| available.get())
        .min(MAX_COPY_WORKERS);
    let mut report = SnapshotCopy { parts: 0, files: 0, bytes: 0 };
    if parts.len() < PARALLEL_THRESHOLD || workers == 1 {
        for part in parts {
            combine_copy_report(
                &mut report, copy_snapshot_part(part, staging, max_files, max_bytes)?,
                max_files, max_bytes,
            )?;
        }
        return Ok(report);
    }
    use std::sync::OnceLock;
    use std::sync::atomic::AtomicUsize;
    let cursor = AtomicUsize::new(0);
    // No need to start additional expensive part copies once an earlier
    // worker has observed a nonrecoverable immutable-source failure.
    let first_failure = AtomicUsize::new(usize::MAX);
    let slots: Vec<OnceLock<Result<SnapshotCopy, PartError>>> =
        (0..parts.len()).map(|_| OnceLock::new()).collect();
    std::thread::scope(|scope| -> Result<(), PartError> {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let cursor = &cursor;
            let slots = &slots;
            let first_failure = &first_failure;
            let handle = std::thread::Builder::new()
                .spawn_scoped(scope, move || loop {
                    let index = cursor.fetch_add(1, Ordering::Relaxed);
                    if index >= parts.len() || index > first_failure.load(Ordering::Acquire) {
                        break;
                    }
                    let result = copy_snapshot_part(&parts[index], staging, max_files, max_bytes);
                    if result.is_err() {
                        first_failure.fetch_min(index, Ordering::AcqRel);
                    }
                    assert!(slots[index].set(result).is_ok(), "part copied more than once");
                })
                .map_err(|error| {
                    PartError::InvalidScan(format!("cannot start snapshot worker: {error}"))
                })?;
            handles.push(handle);
        }
        for handle in handles {
            handle.join().map_err(|_| {
                PartError::InvalidScan("snapshot copy worker panicked".into())
            })?;
        }
        Ok(())
    })?;
    // Monotonic claiming guarantees every index up to the earliest failure
    // was processed. Later slots may intentionally remain uninitialized.
    let first_failure = first_failure.load(Ordering::Acquire);
    for (index, slot) in slots.into_iter().enumerate() {
        if index > first_failure {
            break;
        }
        let part = slot.into_inner()
            .ok_or_else(|| PartError::InvalidScan("missing snapshot copy result".into()))??;
        combine_copy_report(&mut report, part, max_files, max_bytes)?;
    }
    Ok(report)
}

/// Copy a complete active generation into a new directory and validate it
/// before rename. Does not copy catalog schemas or create remote connections.
pub fn copy_frozen_snapshot(
    source: &Path,
    destination: &Path,
    max_parts: usize,
    max_bytes: u64,
) -> Result<SnapshotCopy, PartError> {
    copy_frozen_snapshot_with_file_limit(source, destination, max_parts, usize::MAX, max_bytes)
}

/// Copy a frozen snapshot with a maximum staged file count and byte budget.
pub fn copy_frozen_snapshot_with_file_limit(
    source: &Path,
    destination: &Path,
    max_parts: usize,
    max_files: usize,
    max_bytes: u64,
) -> Result<SnapshotCopy, PartError> {
    let max_files = max_files.min(crate::distributed_inventory::MAX_REPLICA_INVENTORY_FILES);
    let active = resolve_active_snapshot(source)?;
    let parts = query_parts(&active)?;
    if parts.len() > max_parts || max_parts > 16_384 {
        return Err(PartError::InvalidScan(
            "snapshot copy exceeds part limit".into(),
        ));
    }
    // Reject oversized inventories before checksum reads or staging any files.
    crate::distributed_inventory::inventory_active_snapshot(&active, max_files, max_bytes)?;
    inspect_query_source(&active)?;
    ensure_destination_absent(destination)?;
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
    // Keep replica copies outside the source tree. Otherwise a successful
    // copy would silently mutate its supposedly frozen source snapshot.
    let source_root = fs::canonicalize(source).map_err(|error| io_error(source, error))?;
    let destination_parent = fs::canonicalize(parent)
        .map_err(|error| io_error(parent, error))?;
    if destination_parent.starts_with(&source_root) {
        return Err(PartError::InvalidScan(
            "snapshot copy destination must be outside the source tree".into(),
        ));
    }
    let staging = create_snapshot_staging_with(|| {
        parent.join(format!(
            ".rusthouse-snapshot-copy-{}-{}",
            std::process::id(),
            NEXT_COPY.fetch_add(1, Ordering::Relaxed)
        ))
    })
    .map_err(|error| io_error(parent, error))?;
    let copied = copy_snapshot_parts(&parts, &staging, max_files, max_bytes)
        .and_then(|report| {
            inspect_query_source(&staging)?;
            sync_directory(&staging)?;
            Ok(report)
        });
    let report = match copied {
        Ok(report) => report,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if !source_generation_unchanged(source, &active) {
        let _ = fs::remove_dir_all(&staging);
        return Err(PartError::InvalidScan("source snapshot changed during copy".into()));
    }
    if let Err(error) = ensure_destination_absent(destination) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = publish_snapshot_no_replace(&staging, destination) {
        let _ = fs::remove_dir_all(&staging);
        return Err(io_error(destination, error));
    }
    #[cfg(unix)]
    sync_directory(parent)?;
    Ok(report)
}

/// Check a frozen snapshot and report exact copy requirements without writing files.
pub fn preview_frozen_snapshot(
    source: &Path,
    max_parts: usize,
    max_files: usize,
    max_bytes: u64,
) -> Result<SnapshotCopy, PartError> {
    let max_files = max_files.min(crate::distributed_inventory::MAX_REPLICA_INVENTORY_FILES);
    let active = resolve_active_snapshot(source)?;
    let parts = query_parts(&active)?;
    if parts.len() > max_parts || max_parts > 16_384 {
        return Err(PartError::InvalidScan("snapshot preview exceeds part limit".into()));
    }
    inspect_query_source(&active)?;
    let files = crate::distributed_inventory::inventory_snapshot(&active, max_files, max_bytes)?;
    let bytes = files.iter().try_fold(0u64, |total, file| {
        total.checked_add(file.bytes)
            .ok_or_else(|| PartError::InvalidScan("snapshot preview byte overflow".into()))
    })?;
    for part in &parts {
        verify_part_file_hashes(part)?;
        verify_wide_part(part)?;
    }
    if !source_generation_unchanged(source, &active) {
        return Err(PartError::InvalidScan(
            "source snapshot changed during preview".into(),
        ));
    }
    Ok(SnapshotCopy {
        parts: parts.len(),
        files: files.len(),
        bytes,
    })
}

#[cfg(test)]
mod snapshot_staging_collision_tests {
    use super::*;

    #[test]
    fn retries_abandoned_directory_without_touching_its_contents() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-snapshot-staging-collision-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let orphan = root.join("old");
        let new = root.join("new");
        fs::create_dir(&orphan).unwrap();
        fs::write(orphan.join("keep"), b"old copy").unwrap();
        let mut attempts = 0;
        let created = create_snapshot_staging_with(|| {
            attempts += 1;
            if attempts == 1 { orphan.clone() } else { new.clone() }
        }).unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(created, new);
        assert_eq!(fs::read(orphan.join("keep")).unwrap(), b"old copy");
        assert!(fs::metadata(created).unwrap().is_dir());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod parallel_snapshot_copy_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn many_immutable_parts_copy_as_one_atomic_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-parallel-copy-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).unwrap();
        for index in 0..20 {
            write_wide_part(
                &source.join(format!("all_{index}_{index}_0")),
                &[Column { name: "id".into(), data_type: "UInt64".into() }],
                &[vec![Some(index.to_string())]],
            ).unwrap();
        }
        let preview = preview_frozen_snapshot(&source, 20, 256, 4_000_000).unwrap();
        let copied = copy_frozen_snapshot_with_file_limit(
            &source, &destination, 20, 256, 4_000_000,
        ).unwrap();
        assert_eq!(copied, preview);
        assert_eq!(copied.parts, 20);
        assert_eq!(query_parts(&destination).unwrap().len(), 20);
        fs::remove_dir_all(root).unwrap();
    }
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
    fn rejects_copying_into_its_own_snapshot_root() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-snapshot-self-copy-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_wide_part(
            &root.join("all_0_0_0"),
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("1".into())]],
        ).unwrap();
        let nested = root.join("replica");
        assert!(copy_frozen_snapshot(&root, &nested, 1, 1_000_000).is_err());
        assert!(!nested.exists());
        fs::remove_dir_all(root).unwrap();
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

#[cfg(test)]
mod immutable_snapshot_copy_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn copy_does_not_leave_replica_staging_files() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-copy-staging-clean-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        write_wide_part(
            &source.join("all_0_0_0"),
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("8".into())]],
        )
        .unwrap();
        copy_frozen_snapshot(&source, &target, 1, 1_000_000).unwrap();
        for entry in fs::read_dir(target.join("all_0_0_0")).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(!name.to_string_lossy().starts_with(".rusthouse-replica-"));
        }
        fs::remove_dir_all(root).unwrap();
    }
}

/// Catch accidental source snapshot switches before publishing the copied directory.
fn source_generation_unchanged(source: &Path, expected: &Path) -> bool {
    matches!(resolve_active_snapshot(source), Ok(active) if active == expected)
}

#[cfg(test)]
mod source_generation_stability_tests {
    use super::*;

    #[test]
    fn detects_pointer_switch_before_copy_publication() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-copy-generation-switch-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let versions = root.join(".rusthouse-versions");
        let first = versions.join("g1");
        let second = versions.join("g2");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let pointer = root.join(".rusthouse-active");
        fs::write(&pointer, "g1\n").unwrap();
        assert!(source_generation_unchanged(&root, &first));
        fs::write(&pointer, "g2\n").unwrap();
        assert!(!source_generation_unchanged(&root, &first));
        assert!(source_generation_unchanged(&root, &second));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod copy_file_budget_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn refuses_to_publish_a_copy_beyond_its_file_budget() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-copy-file-budget-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).unwrap();
        write_wide_part(
            &source.join("all_0_0_0"),
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("1".into())]],
        ).unwrap();
        assert!(copy_frozen_snapshot_with_file_limit(&source, &destination, 1, 1, 1_000_000).is_err());
        assert!(!destination.exists());
        let copied = copy_frozen_snapshot_with_file_limit(&source, &destination, 1, 32, 1_000_000).unwrap();
        assert!(copied.files > 1);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod preflight_copy_budget_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn rejects_impossible_budget_without_creating_staging_directory() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-copy-preflight-budget-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        write_wide_part(
            &source.join("all_0_0_0"),
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("3".into())]],
        )
        .unwrap();
        let error = copy_frozen_snapshot_with_file_limit(&source, &target, 1, 32, 1)
            .unwrap_err();
        assert!(error.to_string().contains("inventory"));
        assert!(!target.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod pinned_preview_generation_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn previews_selected_generation_instead_of_relisting_the_active_pointer() {
        let root = std::env::temp_dir().join(format!("rusthouse-preview-generations-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = root.join(".rusthouse-versions/g1");
        let second = root.join(".rusthouse-versions/g2");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        write_wide_part(&first.join("all_0_0_0"), &[Column { name: "id".into(), data_type: "UInt64".into() }], &[vec![Some("1".into())]]).unwrap();
        write_wide_part(&second.join("all_0_0_0"), &[Column { name: "id".into(), data_type: "String".into() }], &[vec![Some("data".repeat(256))]]).unwrap();
        fs::write(root.join(".rusthouse-active"), "g1\n").unwrap();
        let first_preview = preview_frozen_snapshot(&root, 1, 100, u64::MAX).unwrap();
        fs::write(root.join(".rusthouse-active"), "g2\n").unwrap();
        let second_preview = preview_frozen_snapshot(&root, 1, 100, u64::MAX).unwrap();
        assert_eq!(first_preview.parts, 1);
        assert_eq!(second_preview.parts, 1);
        assert!(second_preview.bytes > first_preview.bytes);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, unix))]
mod private_snapshot_copy_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn staging_directory_blocks_other_local_users() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-private-copy-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let staging = root.join("staging");
        create_private_staging_directory(&staging).unwrap();
        assert_eq!(
            fs::metadata(&staging).unwrap().permissions().mode() & 0o077,
            0
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, unix))]
mod dangling_destination_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn missing_symlink_target_does_not_make_destination_available() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-dangling-copy-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let destination = root.join("destination");
        assert!(ensure_destination_absent(&destination).is_ok());
        symlink("missing-target", &destination).unwrap();
        assert!(ensure_destination_absent(&destination).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod atomic_publish_tests {
    use super::*;

    #[test]
    fn does_not_replace_destination_created_after_preflight() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-snapshot-no-replace-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let staging = root.join("staging");
        let destination = root.join("published");
        fs::create_dir(&staging).unwrap();
        assert!(ensure_destination_absent(&destination).is_ok());
        // Another actor can create even an empty directory after preflight.
        fs::create_dir(&destination).unwrap();
        assert!(publish_snapshot_no_replace(&staging, &destination).is_err());
        assert!(staging.is_dir());
        assert!(destination.is_dir());
        fs::remove_dir(&destination).unwrap();
        publish_snapshot_no_replace(&staging, &destination).unwrap();
        assert!(!staging.exists());
        assert!(destination.is_dir());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod parallel_copy_failfast_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn failed_source_part_does_not_publish_a_partial_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-copy-cancellation-{}-{}", std::process::id(), line!()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        for index in 0..20 {
            let part = source.join(format!("all_{index}_{index}_0"));
            write_wide_part(
                &part,
                &[Column { name: "id".into(), data_type: "UInt64".into() }],
                &[vec![Some(index.to_string())]],
            ).unwrap();
        }
        let parts = query_parts(&source).unwrap();
        let staging = root.join("staging");
        fs::create_dir(&staging).unwrap();
        fs::write(source.join("all_1_1_0/count.txt"), "9999\n").unwrap();
        assert!(copy_snapshot_parts(&parts, &staging, 10_000, u64::MAX).is_err());
        assert!(!root.join("published").exists());
        fs::remove_dir_all(root).unwrap();
    }
}

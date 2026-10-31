//! Enumerate immutable files that may be copied to another storage node.
use crate::{PartError, query_parts, resolve_active_snapshot};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) const MAX_REPLICA_INVENTORY_FILES: usize = 131_072;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    pub relative_path: PathBuf,
    pub bytes: u64,
}

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_owned(),
        source,
    }
}

fn verified_file_size(path: &Path) -> Result<u64, PartError> {
    // Do not follow links while determining the file type and size.
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if !metadata.file_type().is_file() {
        return Err(PartError::InvalidScan(format!(
            "replication inventory requires regular files: {}",
            path.display()
        )));
    }
    Ok(metadata.len())
}

/// Inventory a frozen table root. Paths are relative to its active generation.
/// Reject links and unexpected directories inside part directories.
pub fn inventory_snapshot(
    root: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<InventoryEntry>, PartError> {
    let active = resolve_active_snapshot(root)?;
    let result = inventory_active_snapshot(&active, max_files, max_bytes)?;
    require_unchanged_generation(root, &active)?;
    Ok(result)
}

fn require_unchanged_generation(root: &Path, active: &Path) -> Result<(), PartError> {
    if resolve_active_snapshot(root)? != active {
        return Err(PartError::InvalidScan(
            "active snapshot changed during inventory".into(),
        ));
    }
    Ok(())
}

pub(crate) fn inventory_active_snapshot(
    active: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<InventoryEntry>, PartError> {
    let parts = query_parts(active)?;
    // Protect callers that request usize::MAX against unbounded inventories.
    let max_files = max_files.min(MAX_REPLICA_INVENTORY_FILES);
    // A typical wide part has several streams plus its metadata files.
    // Cap the estimate to avoid excessive allocations for sparse inventories.
    let estimated_file_capacity = parts.len().saturating_mul(8).min(max_files).min(1024);
    let mut files = Vec::with_capacity(estimated_file_capacity);
    let mut total_bytes = 0u64;
    for part in parts {
        let name = part
            .file_name()
            .ok_or_else(|| PartError::InvalidScan("part has no directory name".into()))?;
        for entry in fs::read_dir(&part).map_err(|error| io_error(&part, error))? {
            let entry = entry.map_err(|error| io_error(&part, error))?;
            let path = entry.path();
            let bytes = verified_file_size(&path)?;
            total_bytes = total_bytes
                .checked_add(bytes)
                .ok_or_else(|| PartError::InvalidScan("inventory size overflow".into()))?;
            if files.len() >= max_files || total_bytes > max_bytes {
                return Err(PartError::InvalidScan(
                    "replication inventory exceeds configured bounds".into(),
                ));
            }
            files.push(InventoryEntry {
                relative_path: PathBuf::from(name).join(entry.file_name()),
                bytes,
            });
        }
    }
    files.sort_unstable_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

/// Stored-byte checksum of one frozen snapshot file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedInventoryEntry {
    pub relative_path: PathBuf,
    pub bytes: u64,
    pub crc32c: u32,
}

/// Build a bounded, content-checksummed inventory for replica reconciliation.
/// CRC32C is an accidental corruption check, not authentication.
fn checksum_inventory_entry(
    active: &Path,
    entry: &InventoryEntry,
) -> Result<CheckedInventoryEntry, PartError> {
    let path = active.join(&entry.relative_path);
    let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
    if !metadata.file_type().is_file() || metadata.len() != entry.bytes {
        return Err(PartError::InvalidScan(
            "snapshot inventory file changed before hashing".into(),
        ));
    }
    let mut file = fs::File::open(&path).map_err(|error| io_error(&path, error))?;
    let opened = file.metadata().map_err(|error| io_error(&path, error))?;
    if !opened.file_type().is_file() || opened.len() != entry.bytes {
        return Err(PartError::InvalidScan(
            "snapshot inventory file changed before hashing".into(),
        ));
    }
    #[cfg(unix)]
    if !same_file_identity(&metadata, &opened) {
        return Err(PartError::InvalidScan(
            "snapshot inventory file replaced before hashing".into(),
        ));
    }
    let (bytes, crc32c) = crate::distributed_crc32c::crc32c_reader(&mut file, entry.bytes)
        .map_err(|error| io_error(&path, error))?;
    if bytes != entry.bytes {
        return Err(PartError::InvalidScan(
            "snapshot inventory file changed during hashing".into(),
        ));
    }
    let final_opened = file.metadata().map_err(|error| io_error(&path, error))?;
    let final_path = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
    if !final_path.file_type().is_file()
        || !same_file_state(&metadata, &final_opened)
        || !same_file_state(&metadata, &final_path)
    {
        return Err(PartError::InvalidScan(
            "snapshot inventory source changed during hashing".into(),
        ));
    }
    Ok(CheckedInventoryEntry {
        relative_path: entry.relative_path.clone(),
        bytes,
        crc32c,
    })
}

pub fn inventory_snapshot_crc32c(
    root: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<CheckedInventoryEntry>, PartError> {
    let active = resolve_active_snapshot(root)?;
    // Inventory exactly the resolved generation without another pointer lookup.
    let entries = inventory_active_snapshot(&active, max_files, max_bytes)?;
    const PARALLEL_THRESHOLD: usize = 64;
    if entries.len() < PARALLEL_THRESHOLD {
        let result = entries
            .iter()
            .map(|entry| checksum_inventory_entry(&active, entry))
            .collect::<Result<Vec<_>, _>>()?;
        require_unchanged_generation(root, &active)?;
        return Ok(result);
    }
    const MAX_HASH_WORKERS: usize = 4;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(MAX_HASH_WORKERS);
    if workers == 1 {
        let result = entries
            .iter()
            .map(|entry| checksum_inventory_entry(&active, entry))
            .collect::<Result<Vec<_>, _>>()?;
        require_unchanged_generation(root, &active)?;
        return Ok(result);
    }

    // Hash each file independently with bounded concurrency. Keep original
    // inventory order, including deterministic first-error reporting.
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cursor = AtomicUsize::new(0);
    // Each cursor index belongs to exactly one worker. Write directly to its
    // one-time result slot instead of buffering another full result vector
    // in each thread and copying all those vectors after join.
    let slots: Vec<OnceLock<Result<CheckedInventoryEntry, PartError>>> =
        (0..entries.len()).map(|_| OnceLock::new()).collect();
    std::thread::scope(|scope| -> Result<(), PartError> {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let cursor = &cursor;
            let entries = &entries;
            let active = &active;
            let slots = &slots;
            let handle = std::thread::Builder::new()
                .spawn_scoped(scope, move || {
                    loop {
                        let index = cursor.fetch_add(1, Ordering::Relaxed);
                        if index >= entries.len() {
                            break;
                        }
                        slots[index]
                            .set(checksum_inventory_entry(active, &entries[index]))
                            .unwrap_or_else(|_| unreachable!("checksum index claimed twice"));
                    }
                })
                .map_err(|error| {
                    PartError::InvalidScan(format!("cannot start checksum worker: {error}"))
                })?;
            handles.push(handle);
        }
        for handle in handles {
            handle.join().map_err(|_| {
                PartError::InvalidScan("snapshot checksum worker panicked".into())
            })?;
        }
        Ok(())
    })?;
    let result = slots
        .into_iter()
        .map(|slot| slot.into_inner().ok_or_else(|| {
            PartError::InvalidScan("missing snapshot checksum result".into())
        })?)
        .collect::<Result<Vec<_>, _>>()?;
    require_unchanged_generation(root, &active)?;
    Ok(result)
}

fn same_file_state(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return same_file_identity(before, after)
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec();
    }
    #[cfg(not(unix))]
    true
}


#[cfg(unix)]
fn same_file_identity(path_metadata: &fs::Metadata, opened_metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    path_metadata.dev() == opened_metadata.dev()
        && path_metadata.ino() == opened_metadata.ino()
}

#[cfg(all(test, unix))]
mod immutable_identity_tests {
    use super::*;

    #[test]
    fn same_length_different_inodes_are_not_the_same_object() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-distributed_inventory-identity-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one");
        let second = root.join("two");
        fs::write(&first, b"abcd").unwrap();
        fs::write(&second, b"wxyz").unwrap();
        let before = fs::symlink_metadata(&first).unwrap();
        let opened = fs::File::open(&second).unwrap().metadata().unwrap();
        assert_eq!(before.len(), opened.len());
        assert!(!same_file_identity(&before, &opened));
        assert!(same_file_identity(&before, &fs::File::open(&first).unwrap().metadata().unwrap()));
        assert!(same_file_state(&before, &before));
        assert!(!same_file_state(&before, &opened));
        fs::write(&first, b"longer").unwrap();
        assert!(!same_file_state(&before, &fs::metadata(&first).unwrap()));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn inventories_real_wide_part_with_bounded_sizes() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-inventory-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("7".into())]],
        )
        .unwrap();
        let files = inventory_snapshot(&root, 32, 1_000_000).unwrap();
        assert!(
            files
                .iter()
                .any(|entry| entry.relative_path == Path::new("all_0_0_0/count.txt"))
        );
        assert!(files.iter().all(|entry| entry.bytes <= 1_000_000));
        assert!(inventory_snapshot(&root, 1, 1_000_000).is_err());
        assert!(inventory_snapshot(&root, 32, 1).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_files() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "rusthouse-inventory-link-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let part = root.join("all_0_0_0");
        fs::create_dir_all(&part).unwrap();
        fs::write(part.join("count.txt"), "0\n").unwrap();
        fs::write(
            part.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` UInt64\n",
        )
        .unwrap();
        symlink(part.join("count.txt"), part.join("shadow.txt")).unwrap();
        assert!(inventory_snapshot(&root, 32, 1_000_000).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_unexpected_directories_inside_parts() {
        let root =
            std::env::temp_dir().join(format!("rusthouse-inventory-dir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("7".into())]],
        )
        .unwrap();
        fs::create_dir(part.join("unexpected")).unwrap();
        assert!(inventory_snapshot(&root, 32, 1_000_000).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod checksummed_inventory_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn parallel_checksums_keep_sorted_paths_and_exact_file_bytes() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-parallel-crc32c-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        for part_number in 0..40 {
            let part = root.join(format!("all_{part_number}_{part_number}_0"));
            write_wide_part(
                &part,
                &[Column { name: "id".into(), data_type: "UInt64".into() }],
                &[vec![Some(part_number.to_string())]],
            ).unwrap();
        }
        let inventory = inventory_snapshot_crc32c(&root, 512, 64 * 1024 * 1024).unwrap();
        assert!(inventory.len() >= 64);
        assert!(inventory.windows(2).all(|pair| {
            pair[0].relative_path < pair[1].relative_path
        }));
        for entry in &inventory {
            let bytes = fs::read(root.join(&entry.relative_path)).unwrap();
            assert_eq!(entry.bytes, bytes.len() as u64);
            assert_eq!(entry.crc32c, crate::distributed_crc32c::crc32c(&bytes));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checksums_actual_wide_part_files() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-hashed-inventory-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("9".into())]],
        ).unwrap();
        let inventory = inventory_snapshot_crc32c(&root, 32, 1_000_000).unwrap();
        let count = inventory
            .iter()
            .find(|item| item.relative_path == Path::new("all_0_0_0/count.txt"))
            .unwrap();
        let expected = fs::read(part.join("count.txt")).unwrap();
        assert_eq!(count.bytes, expected.len() as u64);
        assert_eq!(count.crc32c, crate::distributed_crc32c::crc32c(&expected));
        assert!(inventory_snapshot_crc32c(&root, 1, 1_000_000).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod pinned_inventory_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn resolved_generation_inventory_matches_the_public_root_inventory() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-inventory-single-resolve-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let active = root.join(".rusthouse-versions").join("g1");
        fs::create_dir_all(&active).unwrap();
        write_wide_part(
            &active.join("all_0_0_0"),
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("42".into())]],
        ).unwrap();
        fs::write(root.join(".rusthouse-active"), b"g1\n").unwrap();
        let direct = inventory_active_snapshot(&active, 128, 1_000_000).unwrap();
        let via_root = inventory_snapshot(&root, 128, 1_000_000).unwrap();
        assert_eq!(via_root, direct);
        let checksummed = inventory_snapshot_crc32c(&root, 128, 1_000_000).unwrap();
        assert_eq!(
            checksummed.iter().map(|entry| (&entry.relative_path, entry.bytes)).collect::<Vec<_>>(),
            direct.iter().map(|entry| (&entry.relative_path, entry.bytes)).collect::<Vec<_>>(),
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checksums_follow_one_selected_immutable_generation() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-pinned-inventory-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let versions = root.join(".rusthouse-versions");
        for (generation, value) in [(1, "10"), (2, "20")] {
            let dir = versions.join(format!("g{generation}"));
            fs::create_dir_all(&dir).unwrap();
            write_wide_part(
                &dir.join("all_0_0_0"),
                &[Column { name: "id".into(), data_type: "UInt64".into() }],
                &[vec![Some(value.to_owned())]],
            ).unwrap();
        }
        fs::write(root.join(".rusthouse-active"), "g1\n").unwrap();
        let first = inventory_snapshot_crc32c(&root, 128, 1_000_000).unwrap();
        fs::write(root.join(".rusthouse-active"), "g2\n").unwrap();
        let second = inventory_snapshot_crc32c(&root, 128, 1_000_000).unwrap();
        assert_eq!(first.len(), second.len());
        assert_ne!(first, second);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod small_checksum_inventory_dispatch_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn small_inventories_return_correct_checksums_in_path_order() {
        let root = std::env::temp_dir().join(format!("rusthouse-small-inventory-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_wide_part(&root.join("all_0_0_0"), &[Column { name: "id".into(), data_type: "UInt64".into() }], &[vec![Some("7".into())]]).unwrap();
        let actual = inventory_snapshot_crc32c(&root, 63, 1_000_000).unwrap();
        assert!(!actual.is_empty() && actual.len() < 64);
        assert!(actual.windows(2).all(|window| window[0].relative_path < window[1].relative_path));
        for entry in &actual {
            let data = fs::read(root.join(&entry.relative_path)).unwrap();
            assert_eq!(entry.bytes, data.len() as u64);
            assert_eq!(entry.crc32c, crate::distributed_crc32c::crc32c(&data));
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod checksum_source_version_tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn changing_file_mtime_does_not_require_changing_length() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-inventory-mtime-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let path = root.join("source");
        fs::write(&path, b"abcd").unwrap();
        let file = fs::File::open(&path).unwrap();
        let before = file.metadata().unwrap();
        file.set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        let after = file.metadata().unwrap();
        assert_eq!(before.len(), after.len());
        assert_ne!(before.modified().unwrap(), after.modified().unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod inventory_generation_stability_tests {
    use super::*;

    #[test]
    fn notices_active_pointer_switches_during_inventory() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-inventory-generation-{}-{}", std::process::id(), line!()
        ));
        let _ = fs::remove_dir_all(&root);
        let versions = root.join(".rusthouse-versions");
        let first = versions.join("g1");
        let second = versions.join("g2");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let pointer = root.join(".rusthouse-active");
        fs::write(&pointer, "g1\n").unwrap();
        assert!(require_unchanged_generation(&root, &first).is_ok());
        fs::write(&pointer, "g2\n").unwrap();
        assert!(require_unchanged_generation(&root, &first).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

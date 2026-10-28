//! Enumerate immutable files that may be copied to another storage node.
use crate::{PartError, query_parts, resolve_active_snapshot};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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

/// Inventory a frozen table root. Paths are relative to its active generation.
/// Reject links and unexpected directories inside part directories.
pub fn inventory_snapshot(
    root: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<InventoryEntry>, PartError> {
    let active = resolve_active_snapshot(root)?;
    let parts = query_parts(&active)?;
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    for part in parts {
        let name = part
            .file_name()
            .ok_or_else(|| PartError::InvalidScan("part has no directory name".into()))?;
        for entry in fs::read_dir(&part).map_err(|error| io_error(&part, error))? {
            let entry = entry.map_err(|error| io_error(&part, error))?;
            let path = entry.path();
            // A single no-follow stat checks both file type and byte length.
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| io_error(&path, error))?;
            if !metadata.file_type().is_file() {
                return Err(PartError::InvalidScan(format!(
                    "replication inventory requires regular files: {}",
                    path.display()
                )));
            }
            let bytes = metadata.len();
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
        assert!(files
            .iter()
            .any(|entry| entry.relative_path == Path::new("all_0_0_0/count.txt")));
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
        let root = std::env::temp_dir().join(format!(
            "rusthouse-inventory-dir-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("7".into())]],
        ).unwrap();
        fs::create_dir(part.join("unexpected")).unwrap();
        assert!(inventory_snapshot(&root, 32, 1_000_000).is_err());
        fs::remove_dir_all(root).unwrap();
    }

}
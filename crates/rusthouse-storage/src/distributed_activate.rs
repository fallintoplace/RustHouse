//! Promote a verified, already copied immutable generation on a follower.
//! The caller must hold the local database write lock.
use crate::{
    PartError, inspect_query_source, query_parts, resolve_active_snapshot, verify_part_file_hashes,
};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_PENDING: AtomicU64 = AtomicU64::new(0);

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io { path: path.to_owned(), source }
}

/// Publish only a complete generation newer than the active local pointer.
pub fn activate_verified_generation(root: &Path, generation: u64) -> Result<(), PartError> {
    if generation == 0 {
        return Err(PartError::InvalidScan("generation must be positive".into()));
    }
    let target = root.join(".rusthouse-versions").join(format!("g{generation}"));
    if !fs::symlink_metadata(&target)
        .map_err(|error| io_error(&target, error))?
        .file_type()
        .is_dir()
    {
        return Err(PartError::InvalidScan("generation is not a real directory".into()));
    }
    inspect_query_source(&target)?;
    for part in query_parts(&target)? {
        verify_part_file_hashes(&part)?;
    }
    let pointer = root.join(".rusthouse-active");
    match fs::symlink_metadata(&pointer) {
        Ok(meta) => {
            if !meta.file_type().is_file() || meta.len() > 64 {
                return Err(PartError::InvalidScan("invalid active pointer".into()));
            }
            let current = fs::read_to_string(&pointer)
                .map_err(|error| io_error(&pointer, error))?;
            let prior = current
                .strip_prefix('g')
                .and_then(|text| text.strip_suffix('\n'))
                .and_then(|text| text.parse::<u64>().ok())
                .ok_or_else(|| PartError::InvalidScan("invalid active generation".into()))?;
            if generation <= prior {
                return Err(PartError::InvalidScan("follower generation is not newer".into()));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(&pointer, error)),
    }
    let pending = root.join(format!(
        ".rusthouse-active.replica-{}-{}",
        std::process::id(),
        NEXT_PENDING.fetch_add(1, Ordering::Relaxed)
    ));
    let prepared = (|| -> Result<(), PartError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)
            .map_err(|error| io_error(&pending, error))?;
        file.write_all(format!("g{generation}\n").as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| io_error(&pending, error))
    })();
    if let Err(error) = prepared {
        let _ = fs::remove_file(&pending);
        return Err(error);
    }
    if let Err(error) = fs::rename(&pending, &pointer) {
        let _ = fs::remove_file(&pending);
        return Err(io_error(&pointer, error));
    }
    #[cfg(unix)]
    fs::File::open(root)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error(root, error))?;
    let _ = resolve_active_snapshot(root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn publishes_verified_generation_and_rejects_downgrade() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-activate-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let generation = root.join(".rusthouse-versions/g1");
        fs::create_dir_all(&generation).unwrap();
        write_wide_part(
            &generation.join("all_0_0_0"),
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("12".into())]],
        )
        .unwrap();
        activate_verified_generation(&root, 1).unwrap();
        assert_eq!(resolve_active_snapshot(&root).unwrap(), generation);
        assert!(activate_verified_generation(&root, 1).is_err());
        assert!(activate_verified_generation(&root, 0).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corrupted_generation_does_not_publish_active_pointer() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-activate-corrupt-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let generation = root.join(".rusthouse-versions/g1");
        fs::create_dir_all(&generation).unwrap();
        let part = generation.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("3".into())]],
        ).unwrap();
        let count_file = part.join("count.txt");
        let valid = fs::read(&count_file).unwrap();
        fs::write(&count_file, "999\n").unwrap();
        assert!(activate_verified_generation(&root, 1).is_err());
        assert!(!root.join(".rusthouse-active").exists());
        fs::write(&count_file, valid).unwrap();
        activate_verified_generation(&root, 1).unwrap();
        assert_eq!(resolve_active_snapshot(&root).unwrap(), generation);
        fs::remove_dir_all(root).unwrap();
    }

}
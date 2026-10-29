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
    PartError::Io {
        path: path.to_owned(),
        source,
    }
}

fn validate_fresh_generation(pointer: &Path, generation: u64) -> Result<(), PartError> {
    match fs::symlink_metadata(pointer) {
        Ok(meta) => {
            if !meta.file_type().is_file() || meta.len() > 64 {
                return Err(PartError::InvalidScan("invalid active pointer".into()));
            }
            let current =
                fs::read_to_string(pointer).map_err(|error| io_error(pointer, error))?;
            let prior = current
                .strip_prefix('g')
                .and_then(|text| text.strip_suffix('\n'))
                .and_then(|text| text.parse::<u64>().ok())
                .filter(|&value| value > 0 && current == format!("g{value}\n"))
                .ok_or_else(|| PartError::InvalidScan("invalid active generation".into()))?;
            if generation <= prior {
                return Err(PartError::InvalidScan(
                    "follower generation is not newer".into(),
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(pointer, error)),
    }
    Ok(())
}

/// Publish only a complete generation newer than the active local pointer.
pub fn activate_verified_generation(root: &Path, generation: u64) -> Result<(), PartError> {
    if generation == 0 {
        return Err(PartError::InvalidScan("generation must be positive".into()));
    }
    let target = root
        .join(".rusthouse-versions")
        .join(format!("g{generation}"));
    if !fs::symlink_metadata(&target)
        .map_err(|error| io_error(&target, error))?
        .file_type()
        .is_dir()
    {
        return Err(PartError::InvalidScan(
            "generation is not a real directory".into(),
        ));
    }
    inspect_query_source(&target)?;
    for part in query_parts(&target)? {
        verify_part_file_hashes(&part)?;
    }
    let pointer = root.join(".rusthouse-active");
    validate_fresh_generation(&pointer, generation)?;
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

use std::io::Read;

/// Read the canonical active-generation pointer without opening snapshot parts.
/// A missing pointer is reported as None. This does not validate part contents.
pub fn read_active_generation(root: &Path) -> Result<Option<u64>, PartError> {
    let pointer = root.join(".rusthouse-active");
    match fs::symlink_metadata(&pointer) {
        Ok(meta) if !meta.file_type().is_file() || meta.len() > 64 => {
            return Err(PartError::InvalidScan("invalid active generation pointer".into()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(&pointer, error)),
    }
    let file = fs::File::open(&pointer).map_err(|error| io_error(&pointer, error))?;
    let opened = file.metadata().map_err(|error| io_error(&pointer, error))?;
    if !opened.file_type().is_file() || opened.len() > 64 {
        return Err(PartError::InvalidScan("active generation pointer changed".into()));
    }
    let mut bytes = Vec::new();
    file.take(65)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(&pointer, error))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| PartError::InvalidScan("active generation pointer is not UTF8".into()))?;
    let generation = text
        .strip_prefix('g')
        .and_then(|value| value.strip_suffix('\n'))
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| PartError::InvalidScan("invalid active generation pointer".into()))?;
    if generation == 0 || text != format!("g{generation}\n") {
        return Err(PartError::InvalidScan("noncanonical active generation pointer".into()));
    }
    Ok(Some(generation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn publishes_verified_generation_and_rejects_downgrade() {
        let root = std::env::temp_dir().join(format!("rusthouse-activate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let generation = root.join(".rusthouse-versions/g1");
        fs::create_dir_all(&generation).unwrap();
        write_wide_part(
            &generation.join("all_0_0_0"),
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
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
        let root =
            std::env::temp_dir().join(format!("rusthouse-activate-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let generation = root.join(".rusthouse-versions/g1");
        fs::create_dir_all(&generation).unwrap();
        let part = generation.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("3".into())]],
        )
        .unwrap();
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

#[cfg(test)]
mod active_generation_read_tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_only_valid_canonical_generation_pointers() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-active-read-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert_eq!(read_active_generation(&root).unwrap(), None);
        let pointer = root.join(".rusthouse-active");
        fs::write(&pointer, "g42\n").unwrap();
        assert_eq!(read_active_generation(&root).unwrap(), Some(42));
        for invalid in ["g0\n", "g042\n", "g42", "g1\r\n", "bad\n"] {
            fs::write(&pointer, invalid).unwrap();
            assert!(read_active_generation(&root).is_err(), "{invalid:?}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_active_pointers() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "rusthouse-active-symlink-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("real"), "g1\n").unwrap();
        symlink("real", root.join(".rusthouse-active")).unwrap();
        assert!(read_active_generation(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod promotion_pointer_canonical_tests {
    use super::*;

    #[test]
    fn rejects_noncanonical_active_generation_before_promotion() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-promotion-pointer-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let pointer = root.join(".rusthouse-active");
        for malformed in ["g01\n", "g0\n", "g+1\n", "g1\r\n"] {
            fs::write(&pointer, malformed).unwrap();
            assert!(validate_fresh_generation(&pointer, 2).is_err(), "{malformed:?}");
        }
        fs::write(&pointer, "g1\n").unwrap();
        assert!(validate_fresh_generation(&pointer, 2).is_ok());
        assert!(validate_fresh_generation(&pointer, 1).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

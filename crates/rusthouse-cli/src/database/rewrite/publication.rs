//! Publication protocol for immutable local database snapshots.
//! The caller holds the database write lock. A generation becomes visible
//! only when its fully synced pointer is atomically replaced.

use super::{NEXT_STAGE, data_path};
use super::super::crash_if_requested;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

#[cfg(debug_assertions)]
fn fail_publication_at(point: &str) -> Result<(), String> {
    if std::env::var("RUSTHOUSE_DEBUG_FAIL_AT").is_ok_and(|value| value == point) {
        return Err(format!("injected snapshot publication failure: {point}"));
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn fail_publication_at(_point: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| format!("cannot sync {}: {error}", path.display()))
}

#[cfg(windows)]
pub(super) fn sync_directory(path: &Path) -> Result<(), String> {
    // Windows cannot flush directory handles using std::fs::File::sync_all.
    // These failures previously prevented any snapshot rewrite from being
    // published, even though its immutable part files had been synced.
    // Preserve symlink/file rejection and atomic directory rename. The power-
    // loss durability guarantee remains weaker than on POSIX with fsync.
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.file_type().is_dir() {
        return Err(format!(
            "snapshot path {} is not a real directory",
            path.display()
        ));
    }
    Ok(())
}

fn generations_dir(database: &Path, table: &str) -> Result<std::path::PathBuf, String> {
    let path = data_path(database, table).join(".rusthouse-versions");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            Err("database snapshot directory must not be a symlink or file".into())
        }
        Ok(_) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&path)
                .map_err(|error| format!("cannot create snapshot directory: {error}"))?;
            sync_directory(&data_path(database, table))?;
            Ok(path)
        }
        Err(error) => Err(format!("cannot inspect snapshot directory: {error}")),
    }
}

/// Publish a fresh immutable generation without overwriting a concurrent
/// destination. On Linux the destination absence check and rename are atomic.
#[cfg(target_os = "linux")]
pub(super) fn publish_new_generation(staging: &Path, destination: &Path) -> io::Result<()> {
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
    let path_cstring = |path: &Path| {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in snapshot path"))
    };
    let source = path_cstring(staging)?;
    let target = path_cstring(destination)?;
    let code = unsafe {
        renameat2(
            AT_FDCWD,
            source.as_ptr(),
            AT_FDCWD,
            target.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if code == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Other platforms retain the exclusive database writer-lock requirement.
#[cfg(not(target_os = "linux"))]
pub(super) fn publish_new_generation(staging: &Path, destination: &Path) -> io::Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "snapshot generation exists",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(staging, destination),
        Err(error) => Err(error),
    }
}

/// Publish a verified immutable generation and atomically swap its pointer.
/// The staging writer may stream parts without holding the whole table in memory.
/// Caller must hold the database write lock and validate the existing table.
pub(super) fn publish_snapshot_with(
    database: &Path,
    table: &str,
    prepare: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let versions = generations_dir(database, table)?;
    let mut highest = 0u64;
    for entry in fs::read_dir(&versions).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let label = entry.file_name();
        let Some(label) = label.to_str() else {
            continue;
        };
        if let Some(id) = label
            .strip_prefix('g')
            .and_then(|tail| tail.parse::<u64>().ok())
        {
            highest = highest.max(id);
        }
    }
    let next = highest
        .checked_add(1)
        .ok_or("database snapshot generation overflow")?;
    let name = format!("g{next}");
    let destination = versions.join(&name);
    let staging = versions.join(format!(
        ".pending-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir(&staging)
        .map_err(|error| format!("cannot stage new database snapshot: {error}"))?;
    let prepared = (|| -> Result<(), String> {
        prepare(&staging)?;
        sync_directory(&staging)?;
        crash_if_requested("snapshot-after-staging");
        Ok(())
    })();
    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if destination.exists() {
        let _ = fs::remove_dir_all(&staging);
        return Err("snapshot generation already exists".into());
    }
    let published = fail_publication_at("snapshot-before-generation-rename").and_then(|_| {
        publish_new_generation(&staging, &destination)
            .map_err(|error| format!("cannot publish database generation: {error}"))
    });
    if let Err(error) = published {
        // No generation has become active. Keep the old snapshot readable
        // and do not accumulate failed multi-part staging directories.
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    sync_directory(&versions)?;
    crash_if_requested("snapshot-after-generation");

    let root = data_path(database, table);
    let pointer = root.join(".rusthouse-active");
    let pending = root.join(format!(
        ".rusthouse-active.tmp-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let prepared_pointer = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)
            .map_err(|error| format!("cannot create snapshot pointer: {error}"))?;
        file.write_all(format!("{name}\n").as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("cannot persist snapshot pointer: {error}"))
    })();
    if let Err(error) = prepared_pointer {
        let _ = fs::remove_file(&pending);
        return Err(error);
    }
    crash_if_requested("snapshot-before-pointer");
    let activated = fail_publication_at("snapshot-before-pointer-rename").and_then(|_| {
        fs::rename(&pending, &pointer)
            .map_err(|error| format!("cannot activate database snapshot: {error}"))
    });
    if let Err(error) = activated {
        // The pointer still names the previously committed generation.
        // A newer orphaned generation is retained for offline recovery.
        let _ = fs::remove_file(&pending);
        return Err(error);
    }
    crash_if_requested("snapshot-after-pointer");
    sync_directory(&root)?;
    Ok(())
}

//! Shared local database lock protocol for CLI and HTTP reads.
//! Keep the lock inode in place: unlinking it could admit a second writer.
use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;
#[cfg(not(unix))]
use std::path::PathBuf;

#[cfg(unix)]
mod advisory {
    use std::fs::File;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::raw::c_int;

    unsafe extern "C" {
        fn flock(fd: c_int, operation: c_int) -> c_int;
    }

    const LOCK_SH: c_int = 1;
    const LOCK_EX: c_int = 2;
    const LOCK_NB: c_int = 4;

    pub(super) fn try_lock(file: &File, exclusive: bool) -> io::Result<()> {
        let mode = if exclusive { LOCK_EX } else { LOCK_SH };
        if unsafe { flock(file.as_raw_fd(), mode | LOCK_NB) } == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// Hold this guard until a query finishes consuming its active snapshot.
pub struct DatabaseLock {
    #[cfg(unix)]
    _file: fs::File,
    #[cfg(not(unix))]
    _file: Option<fs::File>,
    #[cfg(not(unix))]
    path: PathBuf,
}

// The non-Unix fallback retains the existing create-new lock-file behavior.
#[cfg(not(unix))]
impl Drop for DatabaseLock {
    fn drop(&mut self) {
        // Windows may refuse deletion while this process still has the
        // create_new lock handle open. Close it before unlinking the lock.
        drop(self._file.take());
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn validate_lock_identity(path: &Path, file: &fs::File) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect database lock {}: {error}", path.display()))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("cannot inspect opened database lock: {error}"))?;
    if !metadata.file_type().is_file()
        || !opened.file_type().is_file()
        || metadata.ino() != opened.ino()
        || metadata.dev() != opened.dev()
    {
        return Err(format!(
            "database lock is not a stable regular file: {}",
            path.display()
        ));
    }
    Ok(())
}

fn format_database_lock_error(action: &str, kind: &str, path: &Path, error: io::Error) -> String {
    format!(
        "cannot {action} database {kind} lock {}: {error}",
        path.display()
    )
}

/// Exclusive for writes, shared for reads. Unix locks are released after crashes.
/// A read never creates the database directory; both modes use the same inode.
pub fn acquire_database_lock(database: &Path, exclusive: bool) -> Result<DatabaseLock, String> {
    if exclusive {
        fs::create_dir_all(database)
            .map_err(|error| format!("cannot create database {}: {error}", database.display()))?;
    }
    let path = database.join(".rusthouse-write.lock");
    let lock_kind = if exclusive { "write" } else { "read" };

    #[cfg(unix)]
    let file = (|| {
        if exclusive {
            return OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path);
        }

        // Existing databases may be on a read-only mount. Open their lock
        // inode without write access. Only a missing lock file needs creation.
        OpenOptions::new().read(true).open(&path).or_else(|error| {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error);
            }
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => Ok(file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    OpenOptions::new().read(true).open(&path)
                }
                Err(error) => Err(error),
            }
        })
    })()
    .map_err(|error| format_database_lock_error("open", lock_kind, &path, error))?;

    #[cfg(unix)]
    {
        validate_lock_identity(&path, &file)?;
        advisory::try_lock(&file, exclusive)
            .map_err(|error| format_database_lock_error("acquire", lock_kind, &path, error))?;
        // A concurrent replacement between open() and flock() must not let
        // two writers lock different inodes under the same path.
        validate_lock_identity(&path, &file)?;
    }

    #[cfg(not(unix))]
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format_database_lock_error("acquire", lock_kind, &path, error))?;

    Ok(DatabaseLock {
        #[cfg(unix)]
        _file: file,
        #[cfg(not(unix))]
        _file: Some(file),
        #[cfg(not(unix))]
        path,
    })
}

#[cfg(all(test, unix))]
mod lock_identity_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn detects_replaced_lock_path_after_open() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-lock-swap-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join(".rusthouse-write.lock");
        fs::write(&path, b"").unwrap();
        let file = fs::File::open(&path).unwrap();
        validate_lock_identity(&path, &file).unwrap();

        fs::rename(&path, dir.join("old-lock")).unwrap();
        fs::write(&path, b"").unwrap();
        assert!(validate_lock_identity(&path, &file).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(all(test, unix))]
mod read_only_lock_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn read_lock_can_open_an_existing_read_only_lock_file() {
        let directory = std::env::temp_dir().join(format!(
            "rusthouse-read-only-db-lock-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        // First reader creates the shared lock inode when it is absent.
        let initial_reader = acquire_database_lock(&directory, false).unwrap();
        drop(initial_reader);

        let path = directory.join(".rusthouse-write.lock");
        let original_mode = fs::metadata(&path).unwrap().permissions().mode();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();

        let reader = acquire_database_lock(&directory, false).unwrap();
        let second_reader = acquire_database_lock(&directory, false).unwrap();
        drop(reader);
        drop(second_reader);

        fs::set_permissions(&path, fs::Permissions::from_mode(original_mode)).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(all(test, not(unix)))]
mod non_unix_lock_release_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn guard_closes_handle_before_unlinking_lock_file() {
        let directory = std::env::temp_dir().join(format!(
            "rusthouse-windows-lock-{}-{}",
            std::process::id(),
            NEXT_PATH.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = fs::remove_dir_all(&directory);
        let lock = acquire_database_lock(&directory, true).unwrap();
        let path = directory.join(".rusthouse-write.lock");
        assert!(path.exists());
        assert!(acquire_database_lock(&directory, true).is_err());
        drop(lock);
        assert!(!path.exists(), "a closed lock must be removable on Windows");

        let retry = acquire_database_lock(&directory, true).unwrap();
        drop(retry);
        assert!(!path.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}

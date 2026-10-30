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
    _file: fs::File,
    #[cfg(not(unix))]
    path: PathBuf,
}

// The non-Unix fallback retains the existing create-new lock-file behavior.
#[cfg(not(unix))]
impl Drop for DatabaseLock {
    fn drop(&mut self) {
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
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| format_database_lock_error("open", lock_kind, &path, error))?;

    #[cfg(unix)]
    {
        validate_lock_identity(&path, &file)?;
        advisory::try_lock(&file, exclusive)
            .map_err(|error| format_database_lock_error("acquire", lock_kind, &path, error))?;
        // Do not keep a lock on a detached inode if the path changed
        // while the advisory lock was being acquired.
        validate_lock_identity(&path, &file)?;
    }

    #[cfg(not(unix))]
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format_database_lock_error("acquire", lock_kind, &path, error))?;

    Ok(DatabaseLock {
        _file: file,
        #[cfg(not(unix))]
        path,
    })
}

#[cfg(all(test, unix))]
mod post_lock_identity_tests {
    use super::*;

    #[test]
    fn detects_replacement_of_the_lock_path_after_open() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-lock-recheck-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let path = root.join(".rusthouse-write.lock");
        fs::write(&path, b"original").unwrap();
        let opened = fs::File::open(&path).unwrap();
        assert!(validate_lock_identity(&path, &opened).is_ok());
        fs::rename(&path, root.join("old-lock")).unwrap();
        fs::write(&path, b"replacement").unwrap();
        assert!(validate_lock_identity(&path, &opened).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

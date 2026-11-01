#!/usr/bin/env python3
"""Cold RustHouse database copy. Stop ALL writers and readers before using."""
import argparse
from contextlib import contextmanager
import math
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile

try:
    import fcntl
except ImportError:
    fcntl = None



def _require_stable_lock(path, fd):
    opened = os.fstat(fd)
    current = os.stat(path, follow_symlinks=False)
    if (not stat.S_ISREG(opened.st_mode) or not stat.S_ISREG(current.st_mode)
            or (opened.st_dev, opened.st_ino) != (current.st_dev, current.st_ino)):
        raise ValueError("database lock path changed while locking")


@contextmanager
def refuse_live_database(source):
    """Hold the database advisory lock for the entire cold backup."""
    lock = source / ".rusthouse-write.lock"
    if fcntl is None:
        if os.name == "posix":
            raise ValueError("cold backup requires POSIX advisory file locking")
        yield
        return
    fd = None
    try:
        # Create the shared advisory lock before copying. A missing lock is
        # not evidence that another process cannot start using the database.
        flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
        fd = os.open(lock, flags, 0o600)
        _require_stable_lock(lock, fd)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise ValueError("database is in use; stop readers and writers") from exc
        # flock protects the opened inode, not a newly substituted pathname.
        _require_stable_lock(lock, fd)
        yield
    finally:
        if fd is not None:
            os.close(fd)


def raise_walk_error(error):
    raise error


def validate_tree(root, max_bytes=None, max_entries=1_000_000):
    """Reject symlinks, special files and oversized source or copied trees."""
    total_bytes = 0
    total_entries = 0
    for base, directories, files in os.walk(root, followlinks=False, onerror=raise_walk_error):
        for item in directories + files:
            total_entries += 1
            if total_entries > max_entries:
                raise ValueError("backup exceeds max_entries preflight budget")
            path = Path(base) / item
            if path.is_symlink():
                raise ValueError("backup refuses symlinks: " + str(path))
        for item in files:
            path = Path(base) / item
            file_stat = path.stat()
            if not stat.S_ISREG(file_stat.st_mode):
                raise ValueError("backup requires regular files: " + str(path))
            if max_bytes is not None:
                total_bytes += file_stat.st_size
                if total_bytes > max_bytes:
                    raise ValueError("backup exceeds max_bytes preflight budget")
    return total_bytes


def copy_regular_file(source, destination):
    """Copy only the exact regular inode checked before opening."""
    source = Path(source)
    before = source.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("backup source file is not regular")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(source, flags)
    with os.fdopen(fd, "rb") as reader:
        opened = os.fstat(reader.fileno())
        if (not stat.S_ISREG(opened.st_mode) or
                (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino)):
            raise ValueError("backup source changed while opening")
        with open(destination, "xb") as writer:
            shutil.copyfileobj(reader, writer, 1024 * 1024)
        finished = os.fstat(reader.fileno())
        if (opened.st_size, opened.st_mtime_ns, opened.st_ctime_ns) != (
                finished.st_size, finished.st_mtime_ns, finished.st_ctime_ns):
            raise ValueError("backup source changed during copying")
    after = source.lstat()
    if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns,
            before.st_ctime_ns) != (after.st_dev, after.st_ino, after.st_size,
                                   after.st_mtime_ns, after.st_ctime_ns):
        raise ValueError("backup source changed during copying")
    shutil.copystat(source, destination, follow_symlinks=False)
    return destination


def sync_copied_tree(root):
    """Persist copied files and directory entries before backup publication."""
    for directory, _subdirs, files in os.walk(root, topdown=False, followlinks=False, onerror=raise_walk_error):
        folder = Path(directory)
        for name in files:
            path = folder / name
            if path.is_symlink() or not path.is_file():
                raise ValueError("backup contains a non-regular file: " + str(path))
            with path.open("rb") as copied:
                os.fsync(copied.fileno())
        if os.name == "posix":
            fd = os.open(folder, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(fd)
            finally:
                os.close(fd)


def sync_parent_directory(path):
    if os.name != "posix":
        return
    fd = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(fd)
    finally:
        os.close(fd)




def preflight(source, destination, max_bytes=None, max_entries=1_000_000):
    if source.is_symlink() or not source.is_dir():
        raise ValueError("source must be an existing real directory")
    source = source.resolve(strict=True)
    if destination.is_symlink():
        raise ValueError("backup destination must not be a symlink")
    # Resolve only for containment checks. Keep the caller's path for
    # publication and reporting: macOS aliases /var to /private/var.
    resolved_destination = destination.resolve(strict=False)
    if (resolved_destination.exists() or resolved_destination == source
            or source in resolved_destination.parents):
        raise ValueError("destination must not exist or be inside the source")
    stage = destination.with_name(destination.name + ".partial")
    if stage.exists() or stage.is_symlink():
        raise ValueError("staging destination already exists: " + str(stage))
    if max_bytes is not None and (type(max_bytes) is not int or max_bytes < 1):
        raise ValueError("max_bytes must be positive")
    if type(max_entries) is not int or not 1 <= max_entries <= 10_000_000:
        raise ValueError("max_entries must be between 1 and 10000000")
    validate_tree(source, max_bytes, max_entries)
    return source, destination, stage


def backup(source, destination, binary="rusthouse", max_bytes=None, doctor_timeout=3600,
           max_entries=1_000_000):
    if (type(doctor_timeout) not in (int, float) or
            not math.isfinite(doctor_timeout) or doctor_timeout <= 0):
        raise ValueError("doctor_timeout must be positive and finite")
    source = Path(source)
    if source.is_symlink() or not source.is_dir():
        raise ValueError("source must be an existing real directory")
    with refuse_live_database(source):
        source, destination, stage = preflight(
            source, destination, max_bytes=max_bytes, max_entries=max_entries
        )
        # Preserve all generations, including hidden files and lock metadata.
        shutil.copytree(source, stage, symlinks=True, copy_function=copy_regular_file)
        # The copied tree may have changed after source preflight.
        validate_tree(stage, max_bytes, max_entries)
        try:
            with tempfile.TemporaryFile() as errors:
                result = subprocess.run(
                    [str(binary), "doctor", "--database", str(stage), "--full-hashes"],
                    stdout=subprocess.DEVNULL, stderr=errors,
                    timeout=doctor_timeout, check=False,
                )
                errors.seek(max(0, errors.tell() - 2000))
                diagnostic = errors.read(2000).decode("utf-8", "replace")
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise RuntimeError("backup remains at " + str(stage) + ": " + str(exc)) from exc
        if result.returncode:
            raise RuntimeError("doctor rejected staged backup at " + str(stage) +
                               ": " + diagnostic)
        sync_copied_tree(stage)
        stage.rename(destination)
        sync_parent_directory(destination)
        return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--binary", default="target/debug/rusthouse")
    parser.add_argument("--offline-confirmed", action="store_true")
    parser.add_argument("--doctor-timeout", type=float, default=3600,
                        help="Maximum seconds to verify the staged backup")
    parser.add_argument("--max-backup-bytes", type=int,
                        help="Reject the source if its files exceed this byte budget")
    parser.add_argument("--max-backup-entries", type=int, default=1_000_000,
                        help="Bound files and directories traversed by preflight")
    args = parser.parse_args()
    if not args.offline_confirmed:
        parser.error("stop ALL RustHouse readers and writers, then pass --offline-confirmed")
    try:
        target = backup(args.source, args.destination, args.binary,
                        max_bytes=args.max_backup_bytes,
                        doctor_timeout=args.doctor_timeout,
                        max_entries=args.max_backup_entries)
    except (ValueError, OSError, RuntimeError, shutil.Error) as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print("Verified cold backup: " + str(target))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

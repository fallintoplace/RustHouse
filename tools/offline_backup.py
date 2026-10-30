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



@contextmanager
def refuse_live_database(source):
    """Hold the database advisory lock for the entire cold backup."""
    lock = source / ".rusthouse-write.lock"
    if fcntl is None or not lock.exists():
        yield
        return
    fd = None
    try:
        flags = os.O_RDWR | getattr(os, "O_NOFOLLOW", 0)
        fd = os.open(lock, flags)
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ValueError("database lock is not a regular file")
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise ValueError("database is in use; stop readers and writers") from exc
        yield
    finally:
        if fd is not None:
            os.close(fd)


def validate_tree(root, max_bytes=None, max_entries=1_000_000):
    """Reject symlinks, special files and oversized source or copied trees."""
    total_bytes = 0
    total_entries = 0
    for base, directories, files in os.walk(root, followlinks=False):
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


def sync_copied_tree(root):
    """Persist copied files and directory entries before backup publication."""
    for directory, _subdirs, files in os.walk(root, topdown=False, followlinks=False):
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
    destination = destination.resolve(strict=False)
    if destination.exists() or destination == source or source in destination.parents:
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
    source, destination, stage = preflight(
        source, destination, max_bytes=max_bytes, max_entries=max_entries
    )
    with refuse_live_database(source):
        # Preserve all generations, including hidden files and lock metadata.
        shutil.copytree(source, stage, symlinks=False, copy_function=shutil.copy2)
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
    except (ValueError, OSError, RuntimeError) as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print("Verified cold backup: " + str(target))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

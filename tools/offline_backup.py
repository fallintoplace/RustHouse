#!/usr/bin/env python3
"""Cold RustHouse database copy. Stop ALL writers and readers before using."""
import argparse
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys


def preflight(source, destination, max_bytes=None):
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
    total_bytes = 0
    for base, directories, files in os.walk(source, followlinks=False):
        for item in directories + files:
            if (Path(base) / item).is_symlink():
                raise ValueError("backup refuses symlinks: " + str(Path(base) / item))
        for item in files:
            file_stat = (Path(base) / item).stat()
            if not stat.S_ISREG(file_stat.st_mode):
                raise ValueError("backup requires regular files: " + str(Path(base) / item))
            if max_bytes is not None:
                total_bytes += file_stat.st_size
                if total_bytes > max_bytes:
                    raise ValueError("backup exceeds max_bytes preflight budget")
    return source, destination, stage


def backup(source, destination, binary="rusthouse", max_bytes=None):
    source, destination, stage = preflight(source, destination, max_bytes=max_bytes)
    # Preserve the complete layout, including hidden files and lock inode contents.
    shutil.copytree(source, stage, symlinks=False, copy_function=shutil.copy2)
    try:
        result = subprocess.run(
            [str(binary), "doctor", "--database", str(stage), "--full-hashes"],
            capture_output=True, text=True, timeout=3600, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise RuntimeError("backup remains at " + str(stage) + ": " + str(exc)) from exc
    if result.returncode:
        raise RuntimeError("doctor rejected staged backup at " + str(stage) +
                           ": " + result.stderr[-2000:])
    stage.rename(destination)
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--binary", default="target/debug/rusthouse")
    parser.add_argument("--offline-confirmed", action="store_true")
    parser.add_argument("--max-backup-bytes", type=int,
                        help="Reject the source if its files exceed this byte budget")
    args = parser.parse_args()
    if not args.offline_confirmed:
        parser.error("stop ALL RustHouse readers and writers, then pass --offline-confirmed")
    try:
        target = backup(args.source, args.destination, args.binary,
                        max_bytes=args.max_backup_bytes)
    except (ValueError, OSError, RuntimeError) as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print("Verified cold backup: " + str(target))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

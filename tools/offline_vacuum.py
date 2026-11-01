#!/usr/bin/env python3
"""Offline generation vacuum for RustHouse local tables. Never run with live readers."""
import argparse
from contextlib import contextmanager
import os
from pathlib import Path
import re
import shutil
import stat
import sys

try:
    import fcntl
except ImportError:
    fcntl = None

MAX_GENERATIONS = 16_384
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]{0,127}\Z")
GENERATION = re.compile(r"g[1-9][0-9]*\Z")


def real_directory(path):
    return not path.is_symlink() and path.is_dir()


def active_label(parts):
    pointer = parts / ".rusthouse-active"
    if pointer.is_symlink() or not pointer.is_file() or pointer.stat().st_size > 64:
        raise ValueError(f"invalid active generation pointer: {pointer}")
    raw = pointer.read_bytes()
    if not raw.endswith(b"\n") or len(raw) > 64:
        raise ValueError(f"invalid active generation pointer: {pointer}")
    label = raw[:-1].decode("ascii")
    if not GENERATION.fullmatch(label) or int(label[1:]) > (1 << 64) - 1:
        raise ValueError(f"noncanonical active generation: {pointer}")
    return label


def plan(database, keep=3):
    database = Path(database)
    if not real_directory(database):
        raise ValueError("database must be an existing real directory")
    if type(keep) is not int or not 2 <= keep <= MAX_GENERATIONS:
        raise ValueError("keep must be between 2 and 16384")
    candidates = []
    for parts in sorted(database.glob("*.parts")):
        name = parts.name.removesuffix(".parts")
        if not IDENTIFIER.fullmatch(name):
            raise ValueError(f"invalid table parts path: {parts}")
        if not real_directory(parts):
            raise ValueError(f"table parts must be real directories: {parts}")
        schema = database / (name + ".schema")
        if not schema.is_file() or schema.is_symlink():
            continue  # Never prune orphaned or partially dropped tables.
        pointer = parts / ".rusthouse-active"
        if not pointer.exists() and not pointer.is_symlink():
            continue  # Legacy single-generation parts.
        active = active_label(parts)
        versions = parts / ".rusthouse-versions"
        if not real_directory(versions):
            raise ValueError(f"invalid generation root: {versions}")
        labels = []
        scanned = 0
        for entry in versions.iterdir():
            scanned += 1
            if scanned > MAX_GENERATIONS:
                raise ValueError(f"invalid generation inventory: {versions}")
            if entry.name.startswith(".pending-"):
                continue
            if not GENERATION.fullmatch(entry.name) or not real_directory(entry):
                raise ValueError(f"unexpected snapshot generation entry: {entry}")
            if int(entry.name[1:]) > (1 << 64) - 1:
                raise ValueError("generation number exceeds u64")
            labels.append(entry.name)
            if len(labels) > MAX_GENERATIONS:
                raise ValueError(f"invalid generation inventory: {versions}")
        if active not in labels:
            raise ValueError(f"invalid generation inventory: {versions}")
        labels.sort(key=lambda label: int(label[1:]))
        retained = set(labels[-keep:])
        retained.add(active)
        for label in labels:
            if label not in retained:
                candidates.append((parts, active, versions / label))
    return candidates


@contextmanager
def exclusive_database_lock(database):
    if fcntl is None or not shutil.rmtree.avoids_symlink_attacks:
        raise ValueError("offline vacuum execute requires safe POSIX flock and rmtree")
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(Path(database) / ".rusthouse-write.lock", flags, 0o600)
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ValueError("database lock is not a regular file")
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise ValueError("database is busy; stop all readers and writers") from exc
        yield
    finally:
        os.close(fd)


def validate_deletion_budget(candidates, max_delete_bytes=None, max_delete_entries=1_000_000):
    if max_delete_bytes is not None and (
            type(max_delete_bytes) is not int or max_delete_bytes < 0):
        raise ValueError("max_delete_bytes must be nonnegative")
    if type(max_delete_entries) is not int or not 1 <= max_delete_entries <= 10_000_000:
        raise ValueError("max_delete_entries must be between 1 and 10000000")
    byte_count = 0
    entry_count = 0

    def raise_walk_error(error):
        raise error

    for _parts, _active, candidate in candidates:
        for directory, dirs, files in os.walk(
                candidate, followlinks=False, onerror=raise_walk_error):
            entry_count += 1
            for name in dirs:
                if (Path(directory) / name).is_symlink():
                    raise ValueError("vacuum refuses symlinked generation entries")
            for name in files:
                entry = (Path(directory) / name).lstat()
                if not stat.S_ISREG(entry.st_mode):
                    raise ValueError("vacuum requires regular generation files")
                entry_count += 1
                byte_count += entry.st_size
            if entry_count > max_delete_entries:
                raise ValueError("vacuum exceeds max_delete_entries")
            if max_delete_bytes is not None and byte_count > max_delete_bytes:
                raise ValueError("vacuum exceeds max_delete_bytes")
    return byte_count, entry_count


def vacuum(database, keep=3, execute=False, offline_confirmed=False,
           max_delete_bytes=None, max_delete_entries=1_000_000):
    database = Path(database)
    if not execute:
        return plan(database, keep)
    if not offline_confirmed:
        raise ValueError("execute requires --offline-confirmed")
    if not real_directory(database):
        raise ValueError("database must be an existing real directory")
    with exclusive_database_lock(database):
        candidates = plan(database, keep)
        validate_deletion_budget(candidates, max_delete_bytes, max_delete_entries)
        for parts, active, candidate in candidates:
            if active_label(parts) != active or not real_directory(candidate):
                raise ValueError("generation changed before offline vacuum deletion")
            shutil.rmtree(candidate)
            folder = candidate.parent
            fd = os.open(folder, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
        return candidates


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--keep", type=int, default=3)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--offline-confirmed", action="store_true")
    parser.add_argument("--max-delete-bytes", type=int)
    parser.add_argument("--max-delete-entries", type=int, default=1_000_000)
    args = parser.parse_args(argv)
    try:
        candidates = vacuum(args.database, args.keep, args.execute, args.offline_confirmed,
                            max_delete_bytes=args.max_delete_bytes,
                            max_delete_entries=args.max_delete_entries)
    except (OSError, ValueError, UnicodeDecodeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    for _parts, _active, path in candidates:
        print(path)
    print(f"{'Deleted' if args.execute else 'Dry-run:'} {len(candidates)} old generations")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

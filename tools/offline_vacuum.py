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
    """Read a bounded, stable active-generation pointer without following links."""
    pointer = parts / ".rusthouse-active"
    before = pointer.lstat()
    if not stat.S_ISREG(before.st_mode) or before.st_size > 64:
        raise ValueError(f"invalid active generation pointer: {pointer}")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(pointer, flags)
    with os.fdopen(fd, "rb") as source:
        opened = os.fstat(source.fileno())
        if (not stat.S_ISREG(opened.st_mode)
                or (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino)):
            raise ValueError(f"active generation pointer changed while opening: {pointer}")
        raw = source.read(65)
        finished = os.fstat(source.fileno())
    after = pointer.lstat()
    identity = lambda info: (info.st_dev, info.st_ino, info.st_size,
                             info.st_mtime_ns, info.st_ctime_ns)
    if identity(before) != identity(opened) or identity(opened) != identity(finished) or identity(finished) != identity(after):
        raise ValueError(f"active generation pointer changed while reading: {pointer}")
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


def vacuum(database, keep=3, execute=False, offline_confirmed=False):
    database = Path(database)
    if not execute:
        return plan(database, keep)
    if not offline_confirmed:
        raise ValueError("execute requires --offline-confirmed")
    if not real_directory(database):
        raise ValueError("database must be an existing real directory")
    with exclusive_database_lock(database):
        candidates = plan(database, keep)
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
    args = parser.parse_args(argv)
    try:
        candidates = vacuum(args.database, args.keep, args.execute, args.offline_confirmed)
    except (OSError, ValueError, UnicodeDecodeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    for _parts, _active, path in candidates:
        print(path)
    print(f"{'Deleted' if args.execute else 'Dry-run:'} {len(candidates)} old generations")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

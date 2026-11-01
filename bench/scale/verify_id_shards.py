"""Verify a contiguous UInt64 TSV manifest in constant working memory.

Usage: python -m bench.scale.verify_id_shards /data/shards/manifest.json
"""
import argparse
import hashlib
import json
import os
import re
import stat
from pathlib import Path

from bench.common import file_change_time_ns

MAX_MANIFEST_BYTES = 8 * 1024 * 1024


def require_uint(value, name, *, zero=False):
    if type(value) is not int or value < (0 if zero else 1):
        raise ValueError(f"{name} must be {'nonnegative' if zero else 'positive'}")
    return value


def expected_shard_bytes(start, rows, step):
    """Exact TSV byte count of a validated UInt64 arithmetic progression.

    Count decimal digit-boundary crossings in O(log10 UInt64), regardless of
    shard row count, instead of allocating or iterating expected rows.
    """
    last = start + (rows - 1) * step
    total = 2 * rows  # One decimal digit and one newline per row.
    threshold = 10
    while threshold <= last:
        first_index = max(0, (threshold - start + step - 1) // step)
        total += rows - min(rows, first_index)
        threshold *= 10
    return total


def verify_file(path, start, rows, step=1):
    if (type(start) is not int or type(rows) is not int or type(step) is not int
            or not 0 <= start <= (1 << 64) - 1
            or rows < 1 or not 1 <= step <= (1 << 64) - 1
            or (rows - 1) > ((1 << 64) - 1 - start) // step):
        raise ValueError("invalid UInt64 shard verification interval")
    path = Path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("shard must be a regular non-symlinked file")
    digest = hashlib.sha256()
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
    with os.fdopen(descriptor, "rb") as source:
        opened = os.fstat(source.fileno())
        if (not stat.S_ISREG(opened.st_mode)
                or (before.st_dev, before.st_ino, before.st_size,
                    before.st_mtime_ns, file_change_time_ns(before)) !=
                    (opened.st_dev, opened.st_ino, opened.st_size,
                     opened.st_mtime_ns, file_change_time_ns(opened))):
            raise ValueError("shard changed before verification")
        expected_size = expected_shard_bytes(start, rows, step)
        if opened.st_size < expected_size:
            raise ValueError(f"{path}: incorrect or missing row data")
        if opened.st_size > expected_size:
            raise ValueError(f"{path}: contains additional rows")
        # Read exact bounded blocks rather than one readline and digest update
        # per UInt64 ID. An oversized/malformed line also fails without an
        # unbounded readline allocation.
        for offset in range(0, rows, 4096):
            count = min(4096, rows - offset)
            first = start + offset * step
            stop = first + count * step
            expected = ("\n".join(map(str, range(first, stop, step))) + "\n").encode("ascii")
            actual = source.read(len(expected))
            if actual != expected:
                raise ValueError(
                    f"{path}: incorrect or missing row near {start + offset * step}"
                )
            digest.update(actual)
        if source.read(1):
            raise ValueError(f"{path}: contains additional rows")
        finished = os.fstat(source.fileno())
        if (opened.st_dev, opened.st_ino, opened.st_size,
                opened.st_mtime_ns, file_change_time_ns(opened)) != (
                finished.st_dev, finished.st_ino, finished.st_size,
                finished.st_mtime_ns, file_change_time_ns(finished)):
            raise ValueError(f"{path}: shard changed during verification")
    after = path.lstat()
    if (before.st_dev, before.st_ino, before.st_size,
            before.st_mtime_ns, file_change_time_ns(before)) != (
            after.st_dev, after.st_ino, after.st_size,
            after.st_mtime_ns, file_change_time_ns(after)):
        raise ValueError(f"{path}: shard changed during verification")
    return digest.hexdigest()


def _manifest_identity(metadata):
    return (metadata.st_dev, metadata.st_ino, metadata.st_size,
            metadata.st_mtime_ns, file_change_time_ns(metadata))


def verify(manifest_path):
    manifest_path = Path(manifest_path)
    before = manifest_path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("refusing non-regular or symlinked shard manifest")
    canonical_path = manifest_path.resolve(strict=True)
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(manifest_path, flags)
    with os.fdopen(descriptor, "rb") as stream:
        opened = os.fstat(stream.fileno())
        if (not stat.S_ISREG(opened.st_mode)
                or _manifest_identity(opened) != _manifest_identity(before)):
            raise ValueError("shard manifest changed before opening")
        payload = stream.read(MAX_MANIFEST_BYTES + 1)
        if len(payload) > MAX_MANIFEST_BYTES:
            raise ValueError("shard manifest exceeds 8 MiB")
        if _manifest_identity(opened) != _manifest_identity(os.fstat(stream.fileno())):
            raise ValueError("shard manifest changed during reading")
    if (_manifest_identity(before) != _manifest_identity(manifest_path.lstat())
            or manifest_path.resolve(strict=True) != canonical_path):
        raise ValueError("shard manifest changed during reading")
    manifest_path = canonical_path
    def unique_fields(pairs):
        result = {}
        for name, value in pairs:
            if name in result:
                raise ValueError(f"duplicate shard manifest JSON field: {name}")
            result[name] = value
        return result

    def reject_nonstandard_number(token):
        raise ValueError(f"nonstandard shard manifest JSON number: {token}")

    manifest = json.loads(payload, object_pairs_hook=unique_fields,
                          parse_constant=reject_nonstandard_number)
    if not isinstance(manifest, dict):
        raise ValueError("shard manifest must be a JSON object")
    # bool == 1 in Python, but a JSON boolean is not a schema version.
    if type(manifest.get("schema_version")) is not int or manifest["schema_version"] != 1:
        raise ValueError("unsupported shard manifest version")
    start = require_uint(manifest.get("start"), "start", zero=True)
    rows = require_uint(manifest.get("rows"), "rows")
    step = require_uint(manifest.get("step", 1), "step")
    if step > (1 << 64) - 1 or (rows - 1) > ((1 << 64) - 1 - start) // step:
        raise ValueError("UInt64 range overflow")
    shards = manifest.get("shards")
    if not isinstance(shards, list) or not 1 <= len(shards) <= 16_384:
        raise ValueError("manifest needs 1..16384 shards")
    position = 0
    seen_paths = set()
    for shard in shards:
        if not isinstance(shard, dict):
            raise ValueError("invalid shard entry")
        shard_start = require_uint(shard.get("start"), "shard start", zero=True)
        count = require_uint(shard.get("rows"), "shard rows")
        path = shard.get("path")
        expected_hash = shard.get("sha256")
        if not isinstance(path, str) or not path:
            raise ValueError("shard path is missing")
        shard_file = Path(path)
        is_relative = not shard_file.is_absolute()
        if is_relative:
            shard_file = manifest_path.parent / shard_file
        if shard_file.is_symlink():
            raise ValueError("shard manifest must not contain symlink paths")
        resolved = shard_file.resolve()
        if is_relative and not resolved.is_relative_to(manifest_path.parent):
            raise ValueError("relative shard path escapes manifest directory")
        canonical = str(resolved)
        if canonical in seen_paths:
            raise ValueError("duplicate shard path")
        seen_paths.add(canonical)
        if shard_start != start + position * step or count > rows - position:
            raise ValueError("gap, overlap or unexpected shard row count")
        if not isinstance(expected_hash, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", expected_hash):
            raise ValueError("missing or invalid shard SHA-256")
        actual = verify_file(shard_file, shard_start, count, step)
        if actual != expected_hash.lower():
            raise ValueError(f"{path}: SHA-256 mismatch")
        position += count
    if position != rows:
        raise ValueError("manifest does not cover all expected rows")
    return {"verified": True, "rows": rows, "step": step, "shards": len(shards)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args(argv)
    try:
        print(json.dumps(verify(args.manifest)))
    except (OSError, ValueError, TypeError, json.JSONDecodeError) as error:
        parser.exit(1, f"shard verification failed: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

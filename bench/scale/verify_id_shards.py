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

MAX_MANIFEST_BYTES = 8 * 1024 * 1024


def require_uint(value, name, *, zero=False):
    if type(value) is not int or value < (0 if zero else 1):
        raise ValueError(f"{name} must be {'nonnegative' if zero else 'positive'}")
    return value


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
                or (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino)):
            raise ValueError("shard changed before verification")
        # Read exact bounded blocks rather than one readline and digest update
        # per UInt64 ID. An oversized/malformed line also fails without an
        # unbounded readline allocation.
        for offset in range(0, rows, 4096):
            count = min(4096, rows - offset)
            expected = "".join(
                f"{start + (offset + index) * step}\n" for index in range(count)
            ).encode("ascii")
            actual = source.read(len(expected))
            if actual != expected:
                raise ValueError(
                    f"{path}: incorrect or missing row near {start + offset * step}"
                )
            digest.update(actual)
        if source.read(1):
            raise ValueError(f"{path}: contains additional rows")
    return digest.hexdigest()


def verify(manifest_path):
    manifest_path = Path(manifest_path)
    if manifest_path.is_symlink():
        raise ValueError("refusing symlinked shard manifest")
    with manifest_path.open("rb") as stream:
        payload = stream.read(MAX_MANIFEST_BYTES + 1)
    if len(payload) > MAX_MANIFEST_BYTES:
        raise ValueError("shard manifest exceeds 8 MiB")
    manifest_path = manifest_path.resolve()
    def unique_fields(pairs):
        result = {}
        for name, value in pairs:
            if name in result:
                raise ValueError(f"duplicate shard manifest JSON field: {name}")
            result[name] = value
        return result

    manifest = json.loads(payload, object_pairs_hook=unique_fields)
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

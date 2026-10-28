"""Verify a contiguous UInt64 TSV manifest in constant working memory.

Usage: python -m bench.scale.verify_id_shards /data/shards/manifest.json
"""
import argparse
import hashlib
import json
from pathlib import Path


def require_uint(value, name, *, zero=False):
    if type(value) is not int or value < (0 if zero else 1):
        raise ValueError(f"{name} must be {'nonnegative' if zero else 'positive'}")
    return value


def verify_file(path, start, rows):
    digest = hashlib.sha256()
    with Path(path).open("rb") as source:
        for expected in range(start, start + rows):
            actual = source.readline()
            if actual != f"{expected}\n".encode("ascii"):
                raise ValueError(f"{path}: incorrect or missing row {expected}")
            digest.update(actual)
        if source.read(1):
            raise ValueError(f"{path}: contains additional rows")
    return digest.hexdigest()


def verify(manifest_path):
    manifest_path = Path(manifest_path).resolve()
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1:
        raise ValueError("unsupported shard manifest version")
    start = require_uint(manifest.get("start"), "start", zero=True)
    rows = require_uint(manifest.get("rows"), "rows")
    if start + rows > 2**64:
        raise ValueError("UInt64 range overflow")
    shards = manifest.get("shards")
    if not isinstance(shards, list) or not shards:
        raise ValueError("manifest needs nonempty shards")
    position = start
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
        if not shard_file.is_absolute():
            shard_file = manifest_path.parent / shard_file
        canonical = str(shard_file.resolve())
        if canonical in seen_paths:
            raise ValueError("duplicate shard path")
        seen_paths.add(canonical)
        if shard_start != position or count > start + rows - position:
            raise ValueError("gap, overlap or unexpected shard row count")
        if not isinstance(expected_hash, str) or len(expected_hash) != 64:
            raise ValueError("missing or invalid shard SHA-256")
        actual = verify_file(shard_file, position, count)
        if actual != expected_hash.lower():
            raise ValueError(f"{path}: SHA-256 mismatch")
        position += count
    if position != start + rows:
        raise ValueError("manifest does not cover all expected rows")
    return {"verified": True, "rows": rows, "shards": len(shards)}


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

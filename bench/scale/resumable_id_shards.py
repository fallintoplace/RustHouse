"""Restartable, independently verified UInt64 TSV shards for large physical imports.

Usage: python -m bench.scale.resumable_id_shards --directory /data/shards --rows 1000000000
Existing shards are never overwritten: they are verified row by row before reuse.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path

MAX_SHARDS = 16_384


def positive(value, name, *, allow_zero=False):
    if type(value) is not int or value < (0 if allow_zero else 1):
        raise ValueError(f"{name} must be {'nonnegative' if allow_zero else 'positive'}")
    return value


def validate_existing(path, start, count):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for value in range(start, start + count):
            expected = f"{value}\n".encode("ascii")
            actual = source.readline()
            if actual != expected:
                raise ValueError(f"invalid or incomplete shard {path} at row {value}")
            digest.update(actual)
        if source.read(1):
            raise ValueError(f"unexpected trailing rows in {path}")
    return digest.hexdigest()


def write_shard(path, start, count):
    if path.is_symlink():
        raise ValueError(f"refusing symlinked shard: {path}")
    if path.exists():
        return validate_existing(path, start, count), True
    pending = path.with_name("." + path.name + f".{os.getpid()}.pending")
    digest = hashlib.sha256()
    try:
        with pending.open("xb", buffering=1024 * 1024) as output:
            # Amortize writes and SHA256 updates for million-row shards without
            # retaining more than one 4096-row block in memory.
            for first in range(start, start + count, 4096):
                last = min(first + 4096, start + count)
                block = "".join(f"{value}\n" for value in range(first, last)).encode("ascii")
                output.write(block)
                digest.update(block)
            output.flush()
            os.fsync(output.fileno())
        # Fail closed rather than replacing an existing shard from a concurrent writer.
        if path.exists():
            pending.unlink()
            return validate_existing(path, start, count), True
        os.link(pending, path)
        pending.unlink()
        return digest.hexdigest(), False
    finally:
        pending.unlink(missing_ok=True)


def create(directory, start, rows, shard_rows):
    positive(start, "start", allow_zero=True)
    positive(rows, "rows")
    positive(shard_rows, "shard_rows")
    if start + rows > 2**64:
        raise ValueError("UInt64 fixture exceeds the supported value domain")
    if (rows - 1) // shard_rows + 1 > MAX_SHARDS:
        raise ValueError("requested shards exceed RustHouse's 16384-part limit")
    directory = Path(directory)
    if directory.is_symlink():
        raise ValueError("refusing symlinked shard directory")
    directory.mkdir(parents=True, exist_ok=True)
    if directory.is_symlink():
        raise ValueError("refusing symlinked shard directory")
    shards = []
    for position in range(start, start + rows, shard_rows):
        count = min(shard_rows, start + rows - position)
        path = directory / f"id-{position:020d}-{position + count:020d}.tsv"
        digest, reused = write_shard(path, position, count)
        shards.append({
            "path": str(path.resolve()), "start": position, "rows": count,
            "sha256": digest, "reused": reused,
        })
    manifest = {
        "schema_version": 1, "start": start, "rows": rows,
        "shard_rows": shard_rows, "shards": shards,
    }
    temporary = directory / f".manifest.{os.getpid()}.pending"
    try:
        with temporary.open("x", encoding="utf-8") as output:
            json.dump(manifest, output, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, directory / "manifest.json")
        if os.name == "posix":
            descriptor = os.open(directory, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
    finally:
        temporary.unlink(missing_ok=True)
    return manifest


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--shard-rows", type=int, default=1_000_000)
    args = parser.parse_args(argv)
    try:
        manifest = create(args.directory, args.start, args.rows, args.shard_rows)
    except (OSError, ValueError) as error:
        parser.exit(1, f"shard generation failed: {error}\n")
    print(json.dumps({"rows": manifest["rows"], "shards": len(manifest["shards"])}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

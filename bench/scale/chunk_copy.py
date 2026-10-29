"""Load sorted headerless TSV shard files with per-file COPY commands.

Each shard is a separate database statement. The complete batch is NOT atomic.
"""
import argparse
import hashlib
import json
import re
import subprocess
import time
from pathlib import Path

MAX_SHARDS = 16_384


def copy_sql(table, path):
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", table):
        raise ValueError("table must be an unquoted SQL identifier")
    escaped = str(Path(path).resolve()).replace("'", "''")
    return f"COPY {table} FROM '{escaped}' WITH (FORMAT TSV)"


def verify_shard(path, entry):
    expected_hash = entry.get("sha256")
    expected_bytes = entry.get("bytes")
    expected_rows = entry.get("rows")
    if not isinstance(expected_hash, str) or not re.fullmatch(r"[0-9a-f]{64}", expected_hash):
        raise ValueError("shard manifest has no valid SHA256")
    if type(expected_bytes) is not int or expected_bytes < 1:
        raise ValueError("shard manifest has invalid byte count")
    if type(expected_rows) is not int or expected_rows < 1:
        raise ValueError("shard manifest has invalid row count")
    digest = hashlib.sha256()
    size = lines = 0
    last_byte = None
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            size += len(block)
            lines += block.count(b"\n")
            last_byte = block[-1]
            digest.update(block)
    if (digest.hexdigest() != expected_hash or size != expected_bytes
            or lines != expected_rows or last_byte != 10):
        raise ValueError(f"shard checksum, byte size, or row count mismatch: {path.name}")


def load_chunks(binary, database, table, source_dir, *, timeout=600, execute=None):
    if timeout <= 0:
        raise ValueError("timeout must be positive")
    source_dir = Path(source_dir)
    manifest = json.loads((source_dir / "manifest.json").read_text(encoding="utf-8"))
    if manifest.get("format") != "rusthouse-tsv-shards-v1" or manifest.get("header") is not False:
        raise ValueError("expected a headerless rusthouse-tsv-shards-v1 manifest")
    shards = manifest.get("shards")
    if not isinstance(shards, list) or not shards:
        raise ValueError("manifest contains no shards")
    if len(shards) > MAX_SHARDS:
        raise ValueError("manifest exceeds RustHouse's 16384-shard limit")
    # Preflight every shard before making the first nontransactional write.
    paths = []
    seen_names = set()
    for entry in shards:
        if not isinstance(entry, dict):
            raise ValueError("invalid shard manifest entry")
        name = entry.get("path")
        if name in seen_names:
            raise ValueError(f"duplicate shard path: {name}")
        seen_names.add(name)
        if not isinstance(name, str) or not re.fullmatch(r"part-[0-9]{6}\.tsv", name):
            raise ValueError("invalid shard path")
        path = source_dir / name
        if not path.is_file() or path.is_symlink():
            raise ValueError(f"missing or unsafe shard: {name}")
        verify_shard(path, entry)
        paths.append(path)
    if manifest.get("total_rows") != sum(e["rows"] for e in shards):
        raise ValueError("total rows mismatch in shard manifest")
    if manifest.get("total_bytes") != sum(e["bytes"] for e in shards):
        raise ValueError("total bytes mismatch in shard manifest")
    run = execute or subprocess.run
    completed = []
    for entry, path in zip(shards, paths):
        name = entry["path"]
        cmd = [str(binary), "sql", copy_sql(table, path), "--database", str(database)]
        started = time.perf_counter()
        outcome = run(cmd, capture_output=True, check=False, timeout=timeout)
        if outcome.returncode:
            raise RuntimeError(
                f"COPY failed after {len(completed)} successful shards; "
                f"failed={name}: {outcome.stderr.decode('utf-8', 'replace')[-500:]}"
            )
        completed.append({"shard": name, "rows": entry["rows"],
                          "seconds": time.perf_counter() - started})
    return {"workload": "tsv-shard-copy", "atomic_batch": False,
            "shards_loaded": len(completed),
            "rows_reported_by_manifest": sum(s["rows"] for s in completed),
            "results": completed}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--table", required=True)
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = load_chunks(args.binary, args.database, args.table,
                             args.source_dir, timeout=args.timeout)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"shard COPY failed: {error}\n")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

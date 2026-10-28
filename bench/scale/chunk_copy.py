"""Load sorted headerless TSV shard files with per-file COPY commands.

Each shard is a separate database statement. The complete batch is NOT atomic.
"""
import argparse
import json
import re
import subprocess
import time
from pathlib import Path


def copy_sql(table, path):
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", table):
        raise ValueError("table must be an unquoted SQL identifier")
    escaped = str(Path(path).resolve()).replace("'", "''")
    return f"COPY {table} FROM '{escaped}' WITH (FORMAT TSV)"


def load_chunks(binary, database, table, source_dir, *, timeout=600, execute=None):
    if timeout <= 0:
        raise ValueError("timeout must be positive")
    manifest = json.loads((Path(source_dir) / "manifest.json").read_text(encoding="utf-8"))
    if manifest.get("format") != "rusthouse-tsv-shards-v1" or manifest.get("header") is not False:
        raise ValueError("expected a headerless rusthouse-tsv-shards-v1 manifest")
    shards = manifest.get("shards")
    if not isinstance(shards, list) or not shards:
        raise ValueError("manifest contains no shards")
    run = execute or subprocess.run
    completed = []
    for entry in shards:
        name = entry.get("path")
        if not isinstance(name, str) or not re.fullmatch(r"part-[0-9]{6}\.tsv", name):
            raise ValueError("invalid shard path")
        path = Path(source_dir) / name
        if not path.is_file() or path.is_symlink():
            raise ValueError(f"missing or unsafe shard: {name}")
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

"""Hash read-only SQL query output spooled to an OS temporary file.

The output stays off the Python heap. The byte limit is checked after the
query finishes; ensure enough temporary disk space for the selected max rows.
"""
import argparse
import hashlib
import json
import math
import re
import subprocess
import tempfile
import time
from pathlib import Path


def query_digest(binary, part, sql, *, expected_sha256=None,
                 max_rows=10000, max_output_bytes=100_000_000,
                 timeout=600, run=None):
    if not 1 <= max_rows <= 250_000:
        raise ValueError("max_rows must be in 1..250000")
    if (type(max_output_bytes) is not int or max_output_bytes < 1
            or type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0):
        raise ValueError("output limit and timeout must be positive and finite")
    if expected_sha256 is not None and not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
        raise ValueError("expected_sha256 must be lowercase SHA256 hex")
    execute = run or subprocess.run
    started = time.perf_counter()
    with tempfile.TemporaryFile(mode="w+b") as output:
        command = [str(binary), "sql", sql, "--part", str(part), "--format",
                   "tsv-no-header", "--max-rows", str(max_rows)]
        result = execute(command, stdout=output, stderr=subprocess.PIPE,
                         check=False, timeout=timeout)
        elapsed = time.perf_counter() - started
        if result.returncode:
            raise RuntimeError(f"query failed: {result.stderr.decode('utf-8', 'replace')[-500:]}")
        output.flush()
        output.seek(0)
        digest = hashlib.sha256()
        size = 0
        while True:
            block = output.read(1024 * 1024)
            if not block:
                break
            size += len(block)
            if size > max_output_bytes:
                raise ValueError("SQL output exceeds max_output_bytes")
            digest.update(block)
    sha = digest.hexdigest()
    if expected_sha256 is not None and sha != expected_sha256:
        raise ValueError("SQL output SHA256 differs from reference")
    return {"workload": "sql-output-digest", "seconds": elapsed,
            "bytes": size, "sha256": sha,
            "verified": expected_sha256 is not None,
            "timing_scope": "fresh CLI query process; OS cache may be warm"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--sql", required=True)
    parser.add_argument("--expected-sha256")
    parser.add_argument("--max-rows", type=int, default=10000)
    parser.add_argument("--max-output-bytes", type=int, default=100_000_000)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = query_digest(args.binary, args.part, args.sql,
                              expected_sha256=args.expected_sha256,
                              max_rows=args.max_rows, max_output_bytes=args.max_output_bytes,
                              timeout=args.timeout)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"SQL digest failed: {error}\n")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

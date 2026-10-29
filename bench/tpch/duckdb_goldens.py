#!/usr/bin/env python3
"""Generate independent TSV references for all 22 full queries with DuckDB CLI."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import math
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

from run import load_queries
from canonical_integrity import verify as verify_canonical_sql


def generate(duckdb_binary, database, queries_dir, output_dir,
             timeout=300, overwrite=False, execute=None, jobs=1, require_canonical=False):
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be positive")
    if type(jobs) is not int or not 1 <= jobs <= 22:
        raise ValueError("jobs must be an integer between 1 and 22")
    if require_canonical:
        verify_canonical_sql(queries_dir)
    queries = load_queries(Path(queries_dir))
    if [number for number, _ in queries] != list(range(1, 23)):
        raise ValueError("full canonical Q1-Q22 SQL corpus required")
    output_dir = Path(output_dir)
    destinations = [output_dir / f"q{n:02d}.tsv" for n, _ in queries]
    if not overwrite and any(path.exists() for path in destinations):
        raise ValueError("refusing to overwrite existing golden output")
    if execute is None:
        execute = lambda cmd: subprocess.run(
            cmd, capture_output=True, timeout=timeout, check=False
        )
    generated = []

    def run_one(item):
        number, sql = item
        cmd = [str(duckdb_binary), "-no-init", "-readonly", "-batch", "-list",
               "-noheader", "-separator", "\t", "-nullvalue", r"\N",
               str(database), "-c", sql]
        result = execute(cmd)
        if result.returncode != 0:
            stderr = result.stderr.decode("utf-8", errors="replace")[-600:]
            raise ValueError(f"q{number:02d}: DuckDB query failed: {stderr}")
        if not isinstance(result.stdout, bytes):
            raise ValueError("DuckDB output must be raw bytes")
        return number, sql, result.stdout

    # Separate readonly DuckDB processes can execute independent queries.
    # Publish nothing unless all of the canonical references succeed.
    with tempfile.TemporaryDirectory(prefix="tpch-full-reference-") as work:
        staging = Path(work)
        if jobs == 1:
            completed = map(run_one, queries)
            for number, sql, output in completed:
                (staging / f"q{number:02d}.tsv").write_bytes(output)
                generated.append({"query": number,
                                  "sql_sha256": hashlib.sha256(sql.encode("utf-8")).hexdigest(),
                                  "reference_sha256": hashlib.sha256(output).hexdigest(),
                                  "bytes": len(output)})
        else:
            with ThreadPoolExecutor(max_workers=jobs) as pool:
                for number, sql, output in pool.map(run_one, queries):
                    (staging / f"q{number:02d}.tsv").write_bytes(output)
                    generated.append({"query": number,
                                      "sql_sha256": hashlib.sha256(sql.encode("utf-8")).hexdigest(),
                                      "reference_sha256": hashlib.sha256(output).hexdigest(),
                                      "bytes": len(output)})
        output_dir.mkdir(parents=True, exist_ok=True)
        for number, _ in queries:
            (staging / f"q{number:02d}.tsv").replace(output_dir / f"q{number:02d}.tsv")
    return {"engine": "duckdb", "queries": generated, "reference_checked": True,
            "logical_input_equivalence_established": False,
            "official_tpch_score": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--duckdb", default="duckdb")
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--queries-dir", type=Path,
                        default=Path(__file__).resolve().parent / "queries")
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--overwrite", action="store_true")
    parser.add_argument("--require-canonical", action="store_true",
                        help="Generate references only from pinned full Q1-Q22 SQL")
    parser.add_argument("--jobs", type=int, default=1,
                        help="Concurrent readonly DuckDB reference queries")
    args = parser.parse_args(argv)
    try:
        report = generate(args.duckdb, args.database, args.queries_dir,
                          args.output_dir, args.timeout, args.overwrite,
                          jobs=args.jobs, require_canonical=args.require_canonical)
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"TPC-H DuckDB golden generation failed: {error}\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

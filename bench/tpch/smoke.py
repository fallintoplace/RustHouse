#!/usr/bin/env python3
"""Run the existing deterministic Q6-inspired fixture through the real CLI.

This is a synthetic compatibility smoke test, not an official TPC-H score.
"""
import argparse
import math
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json
from make_fixture import QUERY, generate
from run import benchmark_one, summarize_coverage


def run_smoke(binary, rows=1000, seed=42, runs=3, warmups=1, timeout=120, execute=None):
    if (type(rows) is not int or type(seed) is not int
            or type(runs) is not int or type(warmups) is not int
            or rows < 1 or runs < 1 or warmups < 0
            or type(timeout) not in (int, float)
            or not math.isfinite(timeout) or timeout <= 0):
        raise ValueError("rows and runs must be positive integers; warmups nonnegative; timeout finite")
    with tempfile.TemporaryDirectory(prefix="rusthouse-tpch-q6-") as directory:
        root = Path(directory)
        fixture = generate(root, rows, seed)
        database = root / "database"
        if execute is None:
            execute = lambda sql: cli_sql(binary, database, sql, timeout)
        for file in ("create.sql", "copy.sql"):
            execute((root / file).read_text(encoding="utf-8").strip().rstrip(";"))
        count = execute("SELECT count(*) FROM lineitem")
        if count.strip() != str(rows).encode("ascii"):
            raise ValueError(f"imported row count differs: expected {rows}, got {count!r}")
        reference = (root / "references" / "q06.tsv").read_bytes()
        result = benchmark_one(6, QUERY, execute, runs, warmups, reference)
        report = {
            "workload": "synthetic-tpch-q6",
            "official_tpch": False,
            "rows": rows, "seed": seed,
            "expected_matches": fixture["matched_rows"],
            "runs": runs, "warmups": warmups,
            "coverage": summarize_coverage([result]),
            "results": [result],
        }
        return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rows", type=int, default=1000)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = run_smoke(args.binary, args.rows, args.seed, args.runs, args.warmups, args.timeout)
    except (OSError, RuntimeError, ValueError) as error:
        parser.exit(1, f"TPC-H Q6 smoke failed: {error}\n")
    emit_json(report, args.output)
    return int(report["coverage"]["failed_queries"] != 0)


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Run a fixed-point Q6 equivalent over converted dbgen LINEITEM data.

RustHouse stores price in cents and discount in basis points. Revenue
is therefore represented in millionths of the original currency unit.
This is query-level validation, not a compliant TPC-H benchmark score.
"""
import argparse
import csv
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json
from run import benchmark_one, summarize_coverage

Q6 = (
    "SELECT SUM(l_extendedprice * l_discount) AS revenue "
    "FROM lineitem WHERE l_shipdate >= '1994-01-01' "
    "AND l_shipdate < '1995-01-01' "
    "AND l_discount BETWEEN 500 AND 700 AND l_quantity < 2400"
)


def expected_revenue(path):
    total = 0
    with Path(path).open("r", encoding="utf-8", newline="") as stream:
        for row in csv.DictReader(stream, delimiter="\t"):
            if ("1994-01-01" <= row["l_shipdate"] < "1995-01-01"
                    and 500 <= int(row["l_discount"]) <= 700
                    and int(row["l_quantity"]) < 2400):
                total += int(row["l_extendedprice"]) * int(row["l_discount"])
    return total


def run_q6(binary, database, lineitem, runs=5, warmups=1, timeout=120, execute=None):
    if runs < 1 or warmups < 0 or timeout <= 0:
        raise ValueError("invalid runs, warmups, or timeout")
    expected = expected_revenue(lineitem)
    # If no rows qualify SUM returns NULL; test it exactly, not a zero.
    reference = (str(expected).encode("ascii") if expected else b"NULL") + b"\n"
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    result = benchmark_one(6, Q6, execute, runs, warmups, reference)
    return {
        "workload": "tpch-q6-fixed-point",
        "official_tpch_score": False,
        "revenue_units": "currency millionths",
        "expected_revenue_integer": expected,
        "coverage": summarize_coverage([result]),
        "results": [result],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--lineitem", type=Path, required=True, help="Converted lineitem.tsv")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = run_q6(args.binary, args.database, args.lineitem,
                        args.runs, args.warmups, args.timeout)
    except (OSError, ValueError, RuntimeError) as error:
        parser.exit(1, f"Q6 verification failed: {error}\n")
    emit_json(report, args.output)
    return int(report["coverage"]["verified_queries"] != 1)


if __name__ == "__main__":
    raise SystemExit(main())

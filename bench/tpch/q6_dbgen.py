#!/usr/bin/env python3
"""Run a fixed-point Q6 equivalent over converted dbgen LINEITEM data.

RustHouse stores price in cents and discount in basis points. Revenue
is therefore represented in millionths of the original currency unit.
This is query-level validation, not a compliant TPC-H benchmark score.
"""
import argparse
import csv
import math
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


def expected_revenue_details(path):
    total = 0
    matched = 0
    with Path(path).open("r", encoding="utf-8", newline="") as stream:
        reader = csv.DictReader(stream, delimiter="\t")
        required = ("l_shipdate", "l_discount", "l_quantity", "l_extendedprice")
        fields = reader.fieldnames or []
        if len(set(fields)) != len(fields) or any(name not in fields for name in required):
            raise ValueError("Q6 input has missing or duplicate required columns")
        for row in reader:
            if None in row or any(row[name] is None for name in fields):
                raise ValueError("Q6 input contains missing or extra data fields")
            if ("1994-01-01" <= row["l_shipdate"] < "1995-01-01"
                    and 500 <= int(row["l_discount"]) <= 700
                    and int(row["l_quantity"]) < 2400):
                total += int(row["l_extendedprice"]) * int(row["l_discount"])
                matched += 1
    return total, matched


def expected_revenue(path):
    return expected_revenue_details(path)[0]


def run_q6(binary, database, lineitem, runs=5, warmups=1, timeout=120, execute=None):
    if (type(runs) is not int or runs < 1
            or type(warmups) is not int or warmups < 0
            or type(timeout) not in (int, float)
            or not math.isfinite(timeout) or timeout <= 0):
        raise ValueError("invalid runs, warmups, or timeout")
    expected, matched = expected_revenue_details(lineitem)
    # SQL SUM returns NULL for empty inputs, not nonempty zero sums.
    reference = (str(expected).encode("ascii") if matched else b"NULL") + b"\n"
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    result = benchmark_one(6, Q6, execute, runs, warmups, reference)
    return {
        "workload": "tpch-q6-fixed-point",
        "official_tpch_score": False,
        "revenue_units": "currency millionths",
        "expected_revenue_integer": expected,
        "matched_rows": matched,
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

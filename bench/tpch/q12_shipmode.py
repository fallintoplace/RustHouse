#!/usr/bin/env python3
"""Verify date-filtered shipment counts by ship mode.

Only the single-key grouping slice of TPC-H Q12 is implemented here.
"""
import argparse
import csv
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json
from run import benchmark_one, summarize_coverage

SQL = ("SELECT l_shipmode, COUNT(*) AS shipments FROM lineitem "
       "WHERE l_receiptdate >= '1994-01-01' AND l_receiptdate < '1995-01-01' "
       "GROUP BY l_shipmode ORDER BY l_shipmode")


def oracle(path):
    counts = {}
    with Path(path).open(encoding="utf-8", newline="") as stream:
        reader = csv.DictReader(stream, delimiter="\t")
        fields = reader.fieldnames or []
        required = ("l_receiptdate", "l_shipmode")
        if len(fields) != len(set(fields)) or any(name not in fields for name in required):
            raise ValueError("Q12 input has missing or duplicate columns")
        for row in reader:
            if None in row or any(row.get(name) is None for name in fields):
                raise ValueError("Q12 input has missing or extra data fields")
            if "1994-01-01" <= row["l_receiptdate"] < "1995-01-01":
                mode = row["l_shipmode"]
                counts[mode] = counts.get(mode, 0) + 1
    return "".join(f"{mode}\t{counts[mode]}\n" for mode in sorted(counts)).encode()


def benchmark(binary, database, lineitem, runs=3, warmups=1, timeout=120, execute=None):
    reference = oracle(lineitem)
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    result = benchmark_one(12, SQL, execute, runs, warmups, reference)
    return {"workload": "tpch-q12-shipmode-subset", "official_tpch": False,
            "coverage": summarize_coverage([result]), "results": [result]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--lineitem", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.runs < 1 or args.warmups < 0 or args.timeout <= 0:
        parser.error("invalid timing parameters")
    report = benchmark(args.binary, args.database, args.lineitem,
                       args.runs, args.warmups, args.timeout)
    emit_json(report, args.output)
    return int(report["coverage"]["verified_queries"] != 1)


if __name__ == "__main__":
    raise SystemExit(main())

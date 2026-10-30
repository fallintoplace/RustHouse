#!/usr/bin/env python3
"""Verified Q1-derived scan totals (no two-key GROUP BY, not official Q1)."""
import argparse
import csv
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json
from run import benchmark_one, summarize_coverage

SQL = ("SELECT COUNT(*) AS count_order, SUM(l_quantity) AS sum_qty, "
       "SUM(l_extendedprice) AS sum_base_price FROM lineitem "
       "WHERE l_shipdate <= '1998-09-02'")


def oracle(path):
    count = quantity = price = 0
    with Path(path).open(encoding="utf-8", newline="") as source:
        reader = csv.DictReader(source, delimiter="\t")
        fields = reader.fieldnames or []
        required = ("l_shipdate", "l_quantity", "l_extendedprice")
        if len(fields) != len(set(fields)) or any(name not in fields for name in required):
            raise ValueError("Q1 input has missing or duplicate columns")
        for row in reader:
            if None in row or any(row.get(name) is None for name in fields):
                raise ValueError("Q1 input has missing or extra data fields")
            if row["l_shipdate"] <= "1998-09-02":
                count += 1
                quantity += int(row["l_quantity"])
                price += int(row["l_extendedprice"])
    return f"{count}\t{quantity if count else 'NULL'}\t{price if count else 'NULL'}\n".encode()


def benchmark(binary, database, source, runs=3, warmups=1, timeout=120, execute=None):
    reference = oracle(source)
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    result = benchmark_one(1, SQL, execute, runs, warmups, reference)
    return {"workload": "tpch-q1-global-subset", "official_tpch": False,
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

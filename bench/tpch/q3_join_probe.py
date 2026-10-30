#!/usr/bin/env python3
"""Run a bounded two-table TPC-H-derived join on small dbgen datasets.

RustHouse currently restricts ORDER BY joins to 16,384 rows per side.
Full-size Q3 additionally needs a three-table join and aggregation.
"""
import argparse
import csv
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json
from run import benchmark_one, summarize_coverage

MAX_ROWS = 16384
SQL = ("SELECT o.o_orderkey, c.c_custkey FROM orders o "
       "JOIN customer c ON o.o_custkey = c.c_custkey "
       "ORDER BY o.o_orderkey LIMIT 20")


def read_table(path, key):
    with Path(path).open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        fields = reader.fieldnames or []
        required = ("o_orderkey", "o_custkey") if key == "o_orderkey" else (key,)
        if len(fields) != len(set(fields)) or any(name not in fields for name in required):
            raise ValueError(f"{path}: missing or duplicate join columns")
        rows = []
        for row in reader:
            if None in row or any(row.get(name) is None for name in fields):
                raise ValueError(f"{path}: missing or extra join fields")
            if len(rows) >= MAX_ROWS:
                raise ValueError(f"{path}: bounded join supports at most {MAX_ROWS} rows")
            rows.append(row)
    return rows


def oracle(orders_path, customers_path):
    customers = set()
    for row in read_table(customers_path, "c_custkey"):
        value = int(row["c_custkey"])
        if value in customers:
            raise ValueError("duplicate customer primary key")
        customers.add(value)
    matched = []
    for row in read_table(orders_path, "o_orderkey"):
        key = int(row["o_custkey"])
        if key in customers:
            matched.append((int(row["o_orderkey"]), key))
    matched.sort()
    return "".join(f"{order}\t{customer}\n" for order, customer in matched[:20]).encode()


def benchmark(binary, database, orders, customers, runs=3, warmups=1, timeout=120, execute=None):
    reference = oracle(orders, customers)
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    result = benchmark_one(3, SQL, execute, runs, warmups, reference)
    return {"workload": "tpch-q3-two-table-join-subset", "official_tpch": False,
            "max_input_rows_per_table": MAX_ROWS,
            "coverage": summarize_coverage([result]), "results": [result]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("binary", "database", "orders", "customers"):
        parser.add_argument("--" + flag, type=Path, required=True)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.runs < 1 or args.warmups < 0 or args.timeout <= 0:
        parser.error("invalid timing parameters")
    report = benchmark(args.binary, args.database, args.orders, args.customers,
                       args.runs, args.warmups, args.timeout)
    emit_json(report, args.output)
    return int(report["coverage"]["verified_queries"] != 1)


if __name__ == "__main__":
    raise SystemExit(main())

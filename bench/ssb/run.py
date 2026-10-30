#!/usr/bin/env python3
"""Run a reference-checked, bounded star-schema join end to end."""
import argparse
import math
import sqlite3
from contextlib import closing
import subprocess
import sys
import tempfile
import time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, latency_summary
from fixture import DATE_KEYS, MAX_FACTS, generate

SQL = ("SELECT l.lo_orderkey, d.d_year FROM lineorder l "
       "JOIN date_dim d ON l.lo_orderdate = d.d_datekey "
       "ORDER BY l.lo_orderkey LIMIT 20")
COUNT_SQL = ("SELECT COUNT(*) FROM lineorder l "
             "JOIN date_dim d ON l.lo_orderdate = d.d_datekey")
GROUPED_SQL = ("SELECT d.d_year, SUM(l.lo_revenue), COUNT(*) FROM lineorder l "
               "JOIN date_dim d ON l.lo_orderdate = d.d_datekey "
               "GROUP BY d.d_year ORDER BY SUM(l.lo_revenue) DESC LIMIT 10")
QUERY_SQL = {
    "sorted-topk": SQL,
    "count-join": COUNT_SQL,
    "grouped-revenue": GROUPED_SQL,
}


def expected_rows(reference, statement=SQL):
    with closing(sqlite3.connect(reference)) as db:
        rows = db.execute(statement).fetchall()
    return "".join(
        "\t".join("NULL" if value is None else str(value) for value in row) + "\n"
        for row in rows
    ).encode()


def benchmark(binary, rows=2000, seed=42, runs=3, warmups=1,
              timeout=120, execute=None, query="sorted-topk"):
    if (type(rows) is not int or not 1 <= rows <= MAX_FACTS
            or type(seed) is not int
            or type(runs) is not int or type(warmups) is not int
            or runs < 1 or warmups < 0 or runs + warmups > 1000
            or type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0 or not isinstance(query, str)
            or query not in QUERY_SQL):
        raise ValueError("invalid row count or timing parameters")
    with tempfile.TemporaryDirectory(prefix="rusthouse-star-bench-") as dirname:
        root = Path(dirname)
        fixture = generate(root, rows=rows, seed=seed)
        statement = QUERY_SQL[query]
        reference = expected_rows(root / "reference.sqlite", statement)
        database = root / "db"
        if execute is None:
            execute = lambda sql: cli_sql(binary, database, sql, timeout)
        for table in ("date_dim", "lineorder"):
            execute((root / (table + ".sql")).read_text().strip().rstrip(";"))
            source = (root / (table + ".tsv")).resolve().as_posix().replace("'", "''")
            execute(f"COPY {table} FROM '{source}' WITH (FORMAT TSV, HEADER TRUE)")
        for table, expected in (("date_dim", len(DATE_KEYS)), ("lineorder", rows)):
            observed = execute(f"SELECT COUNT(*) FROM {table}").strip()
            if observed != str(expected).encode("ascii"):
                raise ValueError(f"{table}: imported {observed!r} rows, expected {expected}")
        samples = []
        status, error = "passed", None
        for iteration in range(-warmups, runs):
            started = time.perf_counter() if iteration >= 0 else None
            output = execute(statement)
            if output != reference:
                status, error = "mismatch", "joined rows differ from SQLite"
                break
            if started is not None:
                samples.append(time.perf_counter() - started)
        if status != "passed":
            samples = []
        return {
            "workload": "ssb-inspired-two-table-join",
            "query": query,
            "official_ssb": False,
            "rows": rows, "seed": seed, "date_dimension_rows": len(DATE_KEYS),
            "runs": runs, "warmups": warmups, "status": status,
            "error": error, "timings_seconds": samples,
            "latency": latency_summary(samples),
        }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rows", type=int, default=2000)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--query", choices=tuple(QUERY_SQL), default="sorted-topk")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = benchmark(args.binary, args.rows, args.seed, args.runs,
                           args.warmups, args.timeout, query=args.query)
    except (OSError, RuntimeError, ValueError, sqlite3.Error,
            subprocess.TimeoutExpired) as error:
        parser.exit(1, f"SSB workload failed: {error}\n")
    emit_json(result, args.output)
    return int(result["status"] != "passed")


if __name__ == "__main__":
    raise SystemExit(main())

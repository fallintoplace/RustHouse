"""End-to-end SQL analytics on reproducible TSV data with a SQLite oracle.

The generated fixture and SQLite query evaluation are excluded from timed samples.
Results are synthetic compatibility/performance observations, not certified scores.
"""
import argparse
import json
import math
import sqlite3
import subprocess
import tempfile
import time
from datetime import date, timedelta
from pathlib import Path

from bench.common import cli_sql, emit_json, latency_summary
from bench.analytics.sqlite_oracle import assert_matches

SCHEMA = (
    "CREATE TABLE events (event_id UInt64, user_id UInt64, region String, "
    "category String, amount UInt64, quantity UInt64, event_date Date, "
    "status String, coupon Nullable(String))"
)
SQLITE_SCHEMA = (
    "CREATE TABLE events (event_id INTEGER, user_id INTEGER, region TEXT, "
    "category TEXT, amount INTEGER, quantity INTEGER, event_date TEXT, "
    "status TEXT, coupon TEXT)"
)
COLUMNS = ("event_id", "user_id", "region", "category", "amount", "quantity", "event_date", "status", "coupon")
ACCOUNTS_SCHEMA = "CREATE TABLE accounts (user_id UInt64, tier String, country String)"
SQLITE_ACCOUNTS_SCHEMA = "CREATE TABLE accounts (user_id INTEGER, tier TEXT, country TEXT)"
ACCOUNTS_ROWS = 997


def load_workload(path):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(data, dict) or not isinstance(data.get("name"), str) or not data["name"].strip():
        raise ValueError("workload must have a nonempty name")
    max_rows = data.get("max_rows", 200_000)
    if type(max_rows) is not int or not 1 <= max_rows <= 200_000:
        raise ValueError("max_rows must be in 1..200000")
    cases = data.get("queries")
    if not isinstance(cases, list) or not 1 <= len(cases) <= 16:
        raise ValueError("workload requires 1..16 queries")
    names = set()
    for case in cases:
        if not isinstance(case, dict) or not isinstance(case.get("name"), str) or not isinstance(case.get("sql"), str):
            raise ValueError("each query needs a name and SQL string")
        name, sql = case["name"], case["sql"].strip()
        if not name or name in names or not sql.upper().startswith("SELECT ") or ";" in sql or len(sql.encode("utf-8")) > 4096:
            raise ValueError("query names must be unique and SQL must be one SELECT under 4096 bytes")
        if case.get("oracle", "exact") not in ("exact", "typed"):
            raise ValueError("query oracle must be exact or typed")
        if "unordered" in case and type(case["unordered"]) is not bool:
            raise ValueError("unordered must be a boolean")
        if case.get("unordered") and case.get("oracle", "exact") != "typed":
            raise ValueError("unordered comparisons require typed oracle")
        names.add(name)
    return data


def fact_row(index):
    return (
        index + 1, (index * 37) % 997,
        ("NA", "EU", "APAC", "LATAM")[index % 4],
        ("books", "games", "music", "tools", "food")[index % 5],
        100 + (index * 53) % 9900, 1 + index % 7,
        (date(2024, 1, 1) + timedelta(days=index % 366)).isoformat(),
        ("paid", "pending", "failed")[index % 3],
        "promo" if index % 4 == 0 else None,
    )


def skewed_fact_row(index):
    """80% of events belong to eight hot users; a long tail still spans 997 IDs.

    Large purchases, dominant regions/products and sparse coupon values are
    deliberately correlated, so scans and group hash tables face real skew.
    The distribution is deterministic and does not depend on random seeds.
    """
    baseline = fact_row(index)
    event_id, _, _, _, _, quantity, day, _, _ = baseline
    user = (index * 13) % 8 if index % 10 < 8 else (index * 37) % 997
    region = "NA" if index % 20 < 15 else ("EU", "APAC", "LATAM")[index % 3]
    category = "books" if index % 10 < 7 else (
        "games", "music", "tools", "food"
    )[index % 4]
    amount = 1_000_000 if index % 257 == 0 else 100 + (index * 53) % 9900
    status = "paid" if index % 10 < 7 else (
        "pending" if index % 2 else "failed"
    )
    coupon = "promo" if index % 11 == 0 else None
    return (event_id, user, region, category, amount, quantity, day, status, coupon)


def make_fixture(directory, rows, *, distribution="uniform"):
    """Stream the RustHouse TSV and feed the same typed rows to SQLite."""
    if type(rows) is not int or not 1 <= rows <= 200_000:
        raise ValueError("rows must be an integer in 1..200000")
    if distribution not in ("uniform", "skewed"):
        raise ValueError("distribution must be uniform or skewed")
    source_row = fact_row if distribution == "uniform" else skewed_fact_row
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / "events.tsv"
    oracle = sqlite3.connect(":memory:")
    oracle.execute(SQLITE_SCHEMA)
    with path.open("w", encoding="utf-8", newline="\n") as stream:
        stream.write("\t".join(COLUMNS) + "\n")
        for start in range(0, rows, 4096):
            batch = [source_row(i) for i in range(start, min(rows, start + 4096))]
            for row in batch:
                stream.write("\t".join(r"\N" if value is None else str(value) for value in row) + "\n")
            oracle.executemany("INSERT INTO events VALUES (?,?,?,?,?,?,?,?,?)", batch)
    oracle.execute(SQLITE_ACCOUNTS_SCHEMA)
    with (directory / "accounts.tsv").open("w", encoding="utf-8", newline="\n") as stream:
        stream.write("user_id\ttier\tcountry\n")
        for account in range(ACCOUNTS_ROWS):
            tier = ("starter", "team", "enterprise")[account % 3]
            country = ("US", "DE", "PL", "VN")[account % 4]
            stream.write(f"{account}\t{tier}\t{country}\n")
            oracle.execute("INSERT INTO accounts VALUES (?,?,?)", (account, tier, country))
    return path, oracle


def sqlite_reference(oracle, sql):
    rows = oracle.execute(sql).fetchmany(100_001)
    if len(rows) > 100_000:
        raise ValueError("oracle result exceeds 100000 rows")
    if any(value is None for row in rows for value in row):
        raise ValueError("workload returns a NULL output: choose an explicit non-null projection")
    rendered = "".join("\t".join(str(value) for value in row) + "\n" for row in rows).encode("utf-8")
    if len(rendered) > 4_000_000:
        raise ValueError("oracle TSV exceeds 4 MB")
    return rendered


def sqlite_typed_reference(oracle, sql):
    rows = oracle.execute(sql).fetchmany(100_001)
    if len(rows) > 100_000:
        raise ValueError("oracle result exceeds 100000 rows")
    return rows


def benchmark_one(number, case, expected, execute, runs, warmups):
    timings = []
    status, error = "passed", None
    for iteration in range(-warmups, runs):
        started = time.perf_counter_ns() if iteration >= 0 else None
        try:
            actual = execute(case["sql"])
        except subprocess.TimeoutExpired:
            status, error = "timeout", "query timed out"
            break
        except (OSError, RuntimeError, ValueError) as failure:
            status, error = "error", str(failure)[:2000]
            break
        elapsed = (time.perf_counter_ns() - started) / 1_000_000_000 if started is not None else None
        if case.get("oracle", "exact") == "typed":
            try:
                assert_matches(actual, expected, unordered=case.get("unordered", False))
            except (ValueError, UnicodeError) as failure:
                status, error = "mismatch", str(failure)[:2000]
                break
        elif actual != expected:
            status, error = "mismatch", f"expected {expected[:120]!r}, received {actual[:120]!r}"
            break
        if elapsed is not None:
            timings.append(elapsed)
    if status != "passed":
        timings = []
    return {
        "query": number, "name": case["name"], "sql": case["sql"],
        "status": status, "timings_seconds": timings,
        "latency": latency_summary(timings), "error": error,
    }


def run_suite(binary, workload, rows=10_000, runs=3, warmups=1, timeout=120,
              execute=None, distribution="uniform"):
    if rows > workload.get("max_rows", 200_000):
        raise ValueError(f"workload accepts at most {workload['max_rows']} rows")
    if (type(runs) is not int or runs < 1
            or type(warmups) is not int or warmups < 0
            or type(timeout) not in (int, float)
            or not math.isfinite(timeout) or timeout <= 0):
        raise ValueError("runs and timeout must be positive; warmups must be nonnegative")
    with tempfile.TemporaryDirectory(prefix="rusthouse-analytics-") as directory:
        root = Path(directory)
        source, oracle = make_fixture(root, rows, distribution=distribution)
        try:
            references = [
                sqlite_typed_reference(oracle, case["sql"])
                if case.get("oracle", "exact") == "typed"
                else sqlite_reference(oracle, case["sql"])
                for case in workload["queries"]
            ]
        finally:
            oracle.close()
        database = root / "database"
        if execute is None:
            execute = lambda sql: cli_sql(binary, database, sql, timeout)
        execute(SCHEMA)
        escaped = source.resolve().as_posix().replace("'", "''")
        execute(f"COPY events FROM '{escaped}' WITH (FORMAT TSV, HEADER TRUE)")
        execute(ACCOUNTS_SCHEMA)
        accounts = (root / "accounts.tsv").resolve().as_posix().replace("'", "''")
        execute(f"COPY accounts FROM '{accounts}' WITH (FORMAT TSV, HEADER TRUE)")
        account_count = execute("SELECT count(*) FROM accounts")
        if account_count.strip() != str(ACCOUNTS_ROWS).encode("ascii"):
            raise ValueError(f"accounts fixture: expected {ACCOUNTS_ROWS}, received {account_count!r}")
        imported = execute("SELECT count(*) FROM events")
        if imported.strip() != str(rows).encode("ascii"):
            raise ValueError(f"fixture row count: expected {rows}, received {imported!r}")
        results = [
            benchmark_one(i, case, expected, execute, runs, warmups)
            for i, (case, expected) in enumerate(zip(workload["queries"], references), 1)
        ]
    verified = sum(case["status"] == "passed" for case in results)
    return {
        "suite": workload["name"], "engine": "rusthouse-cli",
        "source": "synthetic deterministic events", "official_benchmark": False,
        "distribution": distribution,
        "timing_scope": "per-query CLI startup, database scan and TSV output (warm OS cache)",
        "reference_checked": True, "rows": rows, "runs": runs, "warmups": warmups,
        "coverage": {"total_queries": len(results), "verified_queries": verified,
                     "failed_queries": len(results) - verified},
        "results": results,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--workload", required=True, type=Path,
                        help="JSON query pack in bench/analytics/workloads")
    parser.add_argument("--rows", type=int, default=10_000)
    parser.add_argument("--distribution", choices=("uniform", "skewed"), default="uniform")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        workload = load_workload(args.workload)
        report = run_suite(args.binary, workload, args.rows, args.runs,
                           args.warmups, args.timeout, distribution=args.distribution)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired, json.JSONDecodeError) as error:
        parser.exit(1, f"analytics suite failed: {error}\n")
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
    emit_json(report, args.output)
    return int(report["coverage"]["failed_queries"] != 0)


if __name__ == "__main__":
    raise SystemExit(main())

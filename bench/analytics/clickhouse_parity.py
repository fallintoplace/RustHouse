"""Compare RustHouse analytical SQL directly against ClickHouse on identical data.

Needs a local clickhouse-client and a running server. Database names are random;
only the per-run test database is created and dropped. No production tables are
modified. Results, not just execution success, determine parity.
"""
import argparse
import json
import math
import re
import statistics
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

from bench.analytics.sqlite_oracle import assert_matches, cell_equal, parse_rows
from bench.analytics.suite_runner import (
    ACCOUNTS_ROWS, ACCOUNTS_SCHEMA, COLUMNS, SCHEMA, fact_row,
    load_workload, make_fixture,
)
from bench.common import cli_sql, emit_json

INTEGER = re.compile(r"^-?(?:0|[1-9][0-9]*)$")
FLOAT = re.compile(r"^-?(?:(?:[0-9]+\.[0-9]*)|(?:\.[0-9]+)|(?:[0-9]+[eE][+-]?[0-9]+))(?:[eE][+-]?[0-9]+)?$")


def reference_value(value):
    """Keep large integers exact; tolerate only genuine floating result values."""
    if value == r"\N":
        return None
    if INTEGER.fullmatch(value):
        return int(value)
    if FLOAT.fullmatch(value):
        return float(value)
    return value


def assert_clickhouse_parity(rusthouse_output, clickhouse_output, unordered=False):
    """Check every cell, NULL and row multiplicity against a ClickHouse TSV result."""
    expected = [
        tuple(reference_value(cell) for cell in row)
        for row in parse_rows(clickhouse_output)
    ]
    assert_matches(rusthouse_output, expected, unordered=unordered)
    return len(expected)



def _json_result_rows(data):
    """Parse JSONEachRow without silently overwriting duplicate column names."""
    if not isinstance(data, bytes):
        raise TypeError("differential JSON output must be bytes")
    if len(data) > 4_000_000:
        raise ValueError("differential JSON result exceeds 4 MB")

    def no_duplicate_keys(pairs):
        result = {}
        for name, value in pairs:
            if name in result:
                raise ValueError("duplicate JSON result column")
            result[name] = value
        return result

    def no_nonfinite(value):
        raise ValueError(f"non-finite JSON token: {value}")

    rows = []
    for number, line in enumerate(data.splitlines(), 1):
        if not line:
            raise ValueError(f"blank JSON result row {number}")
        row = json.loads(line, object_pairs_hook=no_duplicate_keys,
                         parse_constant=no_nonfinite)
        if not isinstance(row, dict):
            raise ValueError(f"JSON result row {number} is not an object")
        rows.append(row)
    return rows


def _json_cell_equal(actual, expected):
    """RustHouse JSONStringsEachRow emits number text while ClickHouse retains types."""
    if expected is None:
        return actual is None
    if not isinstance(actual, str):
        return False
    if isinstance(expected, str):
        return actual == expected
    if isinstance(expected, bool):
        return actual == ("1" if expected else "0")
    if isinstance(expected, (int, float)):
        return cell_equal(actual, expected)
    return False


def assert_clickhouse_json_parity(rusthouse_output, clickhouse_output, unordered=False):
    """Compare typed ClickHouse JSONEachRow with RustHouse JSONStringsEachRow.

    The reference types decide whether a value is a String, number, or NULL.
    This avoids treating numeric-looking String values as measured numbers.
    """
    actual = _json_result_rows(rusthouse_output)
    expected = _json_result_rows(clickhouse_output)
    if len(actual) != len(expected):
        raise ValueError(
            f"row count mismatch: RustHouse={len(actual)}, ClickHouse={len(expected)}"
        )

    def equal(row, reference):
        return (
            list(row) == list(reference)
            and all(_json_cell_equal(row[key], value) for key, value in reference.items())
        )

    if unordered:
        if len(actual) > 4096:
            raise ValueError("unordered JSON parity is limited to 4096 result rows")
        unmatched = list(actual)
        for reference in expected:
            index = next((i for i, row in enumerate(unmatched) if equal(row, reference)), None)
            if index is None:
                raise ValueError("JSON result multiset differs from ClickHouse")
            unmatched.pop(index)
    else:
        for number, (row, reference) in enumerate(zip(actual, expected), 1):
            if not equal(row, reference):
                raise ValueError(f"JSON result row {number} differs from ClickHouse")
    return len(expected)


def cli_clickhouse(client, host, port, query, timeout, database=None, data=None):
    command = [str(client), "--host", host, "--port", str(port), "--query", query]
    if database:
        command.extend(["--database", database])
    result = subprocess.run(
        command, input=data, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=timeout, check=False,
    )
    if result.returncode:
        raise RuntimeError(
            f"ClickHouse exit {result.returncode}: "
            + result.stderr.decode("utf-8", "replace")[:1000]
        )
    if len(result.stdout) > 4_000_000:
        raise ValueError("ClickHouse result exceeds 4 MB comparison limit")
    return result.stdout


def fixture_json(directory, rows):
    """Reuse the exact deterministic rows loaded into RustHouse, including NULLs."""
    directory = Path(directory)
    path = directory / "events.jsonl"
    with path.open("w", encoding="utf-8") as stream:
        for index in range(rows):
            stream.write(json.dumps(dict(zip(COLUMNS, fact_row(index)))) + "\n")
    accounts_path = directory / "accounts.jsonl"
    with (directory / "accounts.tsv").open(encoding="utf-8") as src, accounts_path.open(
        "w", encoding="utf-8"
    ) as dst:
        next(src)
        for line in src:
            user_id, tier, country = line.rstrip("\n").split("\t")
            dst.write(json.dumps({
                "user_id": int(user_id), "tier": tier, "country": country
            }) + "\n")
    return path, accounts_path


def clickhouse_create_table(schema):
    """The benchmark's declared RustHouse types are native ClickHouse types."""
    if not schema.startswith("CREATE TABLE "):
        raise ValueError("unsupported fixture schema")
    return schema + " ENGINE = Memory"


def paired_query(query, rust, clickhouse, *, runs=5, warmups=2,
                 unordered=False, clock=time.perf_counter_ns):
    """Alternate engine order and verify every paired sample before reporting timings."""
    if runs < 1 or warmups < 0:
        raise ValueError("paired runs must be positive and warmups nonnegative")
    samples = {"clickhouse": [], "rusthouse": []}
    verified_rows = 0
    for iteration in range(-warmups, runs):
        # Alternation reduces systematic first/second-engine cache advantage.
        engines = ("clickhouse", "rusthouse") if iteration % 2 == 0 else ("rusthouse", "clickhouse")
        outputs = {}
        for engine in engines:
            started = clock()
            outputs[engine] = (
                clickhouse(query + " FORMAT JSONEachRow SETTINGS output_format_json_quote_64bit_integers=0")
                if engine == "clickhouse" else rust(query)
            )
            milliseconds = (clock() - started) / 1e6
            if iteration >= 0:
                samples[engine].append(milliseconds)
        verified_rows = assert_clickhouse_json_parity(
            outputs["rusthouse"], outputs["clickhouse"], unordered=unordered
        )
    clickhouse_ms = statistics.median(samples["clickhouse"])
    rusthouse_ms = statistics.median(samples["rusthouse"])
    return {
        "status": "passed",
        "rows": verified_rows,
        "rusthouse_ms": rusthouse_ms,
        "clickhouse_ms": clickhouse_ms,
        "rusthouse_samples_ms": samples["rusthouse"],
        "clickhouse_samples_ms": samples["clickhouse"],
        "clickhouse_over_rusthouse": clickhouse_ms / rusthouse_ms if rusthouse_ms else None,
    }


def run_parity(binary, client, workload, rows=10_000, timeout=120,
               host="localhost", port=9000, runs=5, warmups=2):
    if not 1 <= rows <= min(200_000, workload.get("max_rows", 200_000)):
        raise ValueError("fixture rows exceed workload range")
    if timeout <= 0 or not 1 <= port <= 65535:
        raise ValueError("invalid ClickHouse timeout or port")
    db_name = "rusthouse_analytics_" + uuid.uuid4().hex[:16]
    results = []
    created = False
    with tempfile.TemporaryDirectory(prefix="rusthouse-ch-parity-") as temp:
        root = Path(temp)
        source, sqlite = make_fixture(root, rows)
        sqlite.close()
        event_json, account_json = fixture_json(root, rows)
        database = root / "rusthouse-db"
        rust = lambda sql: cli_sql(binary, database, sql, timeout)
        rust_json = lambda sql: cli_sql(binary, database, sql, timeout, select_format="jsonl")
        ch = lambda sql, data=None, database_name=None: cli_clickhouse(
            client, host, port, sql, timeout, database_name, data
        )
        try:
            ch(f"CREATE DATABASE {db_name}")
            created = True
            ch(clickhouse_create_table(SCHEMA), database_name=db_name)
            ch(clickhouse_create_table(ACCOUNTS_SCHEMA), database_name=db_name)
            ch("INSERT INTO events FORMAT JSONEachRow",
               event_json.read_bytes(), db_name)
            ch("INSERT INTO accounts FORMAT JSONEachRow",
               account_json.read_bytes(), db_name)
            rust(SCHEMA)
            rust(ACCOUNTS_SCHEMA)
            escaped = source.resolve().as_posix().replace("'", "''")
            rust(f"COPY events FROM '{escaped}' WITH (FORMAT TSV, HEADER TRUE)")
            accounts = (root / "accounts.tsv").resolve().as_posix().replace("'", "''")
            rust(f"COPY accounts FROM '{accounts}' WITH (FORMAT TSV, HEADER TRUE)")
            for name, expected in (("events", rows), ("accounts", ACCOUNTS_ROWS)):
                for engine, run in (("clickhouse", lambda query: ch(query, database_name=db_name)),
                                    ("rusthouse", rust)):
                    actual = run(f"SELECT count(*) FROM {name}").strip()
                    if actual != str(expected).encode():
                        raise ValueError(f"{engine} fixture {name} count mismatch: {actual!r}")
            for case in workload["queries"]:
                try:
                    measured = paired_query(
                        case["sql"], rust_json,
                        lambda query: ch(query, database_name=db_name),
                        runs=runs, warmups=warmups,
                        unordered=case.get("unordered", False),
                    )
                    results.append({"name": case["name"], **measured})
                except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
                    results.append({
                        "name": case["name"], "status": "failed",
                        "error": str(error)[:1000],
                    })
        finally:
            if created:
                ch(f"DROP DATABASE IF EXISTS {db_name} SYNC")
    verified = sum(item["status"] == "passed" for item in results)
    return {
        "suite": workload["name"], "engines": ["rusthouse", "clickhouse"],
        "rows": rows, "runs": runs, "warmups": warmups,
        "timing": "paired interleaved runs, median wall clock including CLI overhead",
        "official_benchmark": False,
        "fixture": "deterministic identical JSONEachRow and TSV data",
        "oracle": "typed ClickHouse JSONEachRow vs RustHouse JSONStringsEachRow",
        "coverage": {
            "total_queries": len(results), "verified_queries": verified,
            "failed_queries": len(results) - verified,
        },
        "results": results,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--client", default="clickhouse-client", type=Path)
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=9000)
    parser.add_argument("--workload", required=True, type=Path)
    parser.add_argument("--rows", type=int, default=10_000)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    workload = load_workload(args.workload)
    report = run_parity(args.binary, args.client, workload, args.rows,
                        args.timeout, args.host, args.port, args.runs, args.warmups)
    emit_json(report, args.output)
    return int(report["coverage"]["failed_queries"] != 0)


if __name__ == "__main__":
    raise SystemExit(main())

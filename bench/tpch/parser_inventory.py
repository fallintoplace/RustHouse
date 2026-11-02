#!/usr/bin/env python3
"""Check whether RustHouse parses each canonical TPC-H query without loading data.

A parsed query is not necessarily executable or correct. Use official_coverage
with an independent reference to measure end-to-end correctness.
"""
import argparse
import json
import math
import subprocess
import tempfile
from pathlib import Path

from official_coverage import TPCH_QUERY_COUNT, clean_sql

MAX_AST_BYTES = 2 * 1024 * 1024


def parse_statement(binary, sql, timeout=10):
    """Run the public --show-ast CLI path with a bounded output buffer."""
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        try:
            completed = subprocess.run(
                [str(binary), "sql", sql, "--database", ".", "--show-ast"],
                stdout=output, stderr=errors, check=False, timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            return False, "parser timed out"
        output.seek(0, 2)
        if output.tell() > MAX_AST_BYTES:
            return False, "parser output exceeds 2 MiB"
        if completed.returncode:
            errors.seek(0)
            return False, errors.read(500).decode("utf-8", "replace").strip()
        return True, None


def inventory(binary, queries_dir, *, parse=None):
    """Inventory all 22 canonical files; never report a missing query as parsed."""
    if parse is None:
        parse = lambda sql: parse_statement(binary, sql)
    results = []
    for query in range(1, TPCH_QUERY_COUNT + 1):
        path = Path(queries_dir) / f"q{query:02d}.sql"
        if not path.is_file():
            results.append({"query": query, "status": "missing_query"})
            continue
        try:
            sql = clean_sql(path)
            if not sql:
                raise ValueError("empty query")
            parsed, error = parse(sql)
        except (OSError, UnicodeError, ValueError) as exc:
            results.append({"query": query, "status": "invalid_query", "error": str(exc)[:500]})
            continue
        results.append({"query": query, "status": "parsed" if parsed else "parse_error",
                        **({"error": str(error or "unknown parser error")[:500]} if not parsed else {})})
    passed = sum(item["status"] == "parsed" for item in results)
    return {
        "workload": "tpch-canonical-parser-inventory",
        "official_tpch_score": False,
        "parsed_queries": passed,
        "total_queries": TPCH_QUERY_COUNT,
        "results": results,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--queries-dir", type=Path, default=Path(__file__).parent / "queries")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    report = inventory(args.binary, args.queries_dir)
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return int(report["parsed_queries"] != TPCH_QUERY_COUNT)


if __name__ == "__main__":
    raise SystemExit(main())

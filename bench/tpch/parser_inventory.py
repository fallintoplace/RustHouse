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
from run import query_fingerprint
from canonical_integrity import verify as verify_canonical_sql

MAX_AST_BYTES = 2 * 1024 * 1024
SPECIAL_HANDLER_AST_SUFFIX = " cannot use --show-ast or --explain yet"


def recognized_by_special_handler(error):
    """A query handler parsed the SQL but does not expose its plan as an AST."""
    return (isinstance(error, str) and error.startswith("error: ")
            and error.endswith(SPECIAL_HANDLER_AST_SUFFIX))


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
        ast_bytes = output.tell()
        if ast_bytes > MAX_AST_BYTES:
            return False, "parser output exceeds 2 MiB"
        if completed.returncode:
            errors.seek(0)
            return False, errors.read(500).decode("utf-8", "replace").strip()
        if ast_bytes == 0:
            return False, "parser returned success without an AST"
        return True, None


def inventory(binary, queries_dir, *, parse=None, require_canonical=False):
    """Inventory all 22 canonical files; never report a missing query as parsed."""
    if require_canonical:
        verify_canonical_sql(queries_dir)
    if parse is None:
        parse = lambda sql: parse_statement(binary, sql)
    results = []
    loaded_sql = []
    for query in range(1, TPCH_QUERY_COUNT + 1):
        path = Path(queries_dir) / f"q{query:02d}.sql"
        if not path.is_file():
            results.append({"query": query, "status": "missing_query"})
            continue
        try:
            sql = clean_sql(path)
            if not sql:
                raise ValueError("empty query")
            loaded_sql.append((query, sql))
            parsed, error = parse(sql)
        except (OSError, UnicodeError, ValueError) as exc:
            results.append({"query": query, "status": "invalid_query", "error": str(exc)[:500]})
            continue
        if parsed:
            results.append({"query": query, "status": "parsed"})
        elif recognized_by_special_handler(error):
            results.append({"query": query, "status": "recognized_by_special_handler",
                            "detail": str(error)[:500]})
        else:
            results.append({"query": query, "status": "parse_error",
                            "error": str(error or "unknown parser error")[:500]})
    passed = sum(item["status"] == "parsed" for item in results)
    special = sum(item["status"] == "recognized_by_special_handler" for item in results)
    first_errors = {}
    for item in results:
        if item["status"] == "parse_error":
            first_errors.setdefault(item["error"], []).append(item["query"])
    error_groups = [
        {"error": error, "count": len(queries), "queries": queries}
        for error, queries in sorted(first_errors.items(),
                                     key=lambda pair: (-len(pair[1]), pair[0]))
    ]
    return {
        "workload": "tpch-canonical-parser-inventory",
        "official_tpch_score": False,
        "parsed_queries": passed,
        "special_handler_queries": special,
        "recognized_queries": passed + special,
        "total_queries": TPCH_QUERY_COUNT,
        "query_sha256": query_fingerprint(loaded_sql),
        "first_error_groups": error_groups,
        "results": results,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--queries-dir", type=Path, default=Path(__file__).parent / "queries")
    parser.add_argument("--require-canonical", action="store_true",
                        help="Verify all 22 files match pinned upstream SQL")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    report = inventory(args.binary, args.queries_dir,
                       require_canonical=args.require_canonical)
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return int(report["recognized_queries"] != TPCH_QUERY_COUNT)


if __name__ == "__main__":
    raise SystemExit(main())

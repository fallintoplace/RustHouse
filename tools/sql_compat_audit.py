#!/usr/bin/env python3
"""Conservative parser-only SQL compatibility audit. Never executes SELECT."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def parse_lines(content):
    queries = []
    for number, line in enumerate(content.splitlines(), 1):
        value = line.strip()
        if not value or value.startswith("--"):
            continue
        if value.endswith(";"):
            value = value[:-1].rstrip()
        if (not re.match(r"(?i)^SELECT\s", value) or ";" in value
                or len(value.encode("utf-8")) > 4096):
            raise ValueError("line " + str(number) + ": one bounded SELECT is required")
        if len(queries) >= 200:
            raise ValueError("at most 200 SELECT queries per audit")
        queries.append((number, value))
    if not queries:
        raise ValueError("no SELECT queries provided")
    return queries


def audit(binary, queries, run=subprocess.run, fail_fast=False):
    results = []
    with tempfile.TemporaryDirectory(prefix="rusthouse-sql-audit-") as database:
        for line, query in queries:
            try:
                # A malformed or noisy CLI can emit unbounded output.
                # Spool it to disk and only read the bounded failure reason.
                with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
                    result = run(
                        [str(binary), "sql", query, "--database", database, "--explain"],
                        stdout=stdout, stderr=stderr, timeout=10, check=False,
                    )
                    status = result.returncode == 0
                    stderr.seek(0)
                    reason = "" if status else stderr.read(500).decode("utf-8", "replace").strip()
            except (OSError, subprocess.TimeoutExpired) as exc:
                status = False
                reason = str(exc)[:500]
            results.append({"line": line, "parser_accepted": status, "reason": reason})
            if fail_fast and not status:
                break
    return {
        "schema_version": 1,
        "scope": "parser-only, no data read or SQL execution",
        "total": len(results),
        "requested": len(queries),
        "stopped_early": len(results) < len(queries),
        "accepted": sum(row["parser_accepted"] for row in results),
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("queries", type=Path, help="one SELECT per line")
    parser.add_argument("--binary", default="target/debug/rusthouse")
    parser.add_argument("--fail-fast", action="store_true")
    args = parser.parse_args()
    try:
        statements = parse_lines(args.queries.read_text(encoding="utf-8"))
        result = audit(args.binary, statements, fail_fast=args.fail_fast)
    except (OSError, UnicodeError, ValueError) as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2))
    return 0 if result["total"] == result["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

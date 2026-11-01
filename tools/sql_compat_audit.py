#!/usr/bin/env python3
"""Conservative parser-only SQL compatibility audit. Never executes SELECT."""
import argparse
import json
import math
import os
from pathlib import Path
import re
import subprocess
import stat
import sys
import tempfile


def _has_separator(sql):
    quote = None
    index = 0
    while index < len(sql):
        char = sql[index]
        if quote is not None:
            if char == "\\":
                index += 2
                continue
            if char == quote:
                if index + 1 < len(sql) and sql[index + 1] == quote:
                    index += 2
                    continue
                quote = None
        elif sql.startswith("--", index) or char == "#":
            break
        elif sql.startswith("/*", index):
            end = sql.find("*/", index + 2)
            if end == -1:
                return True
            index = end + 2
            continue
        elif char in ("'", '"'):
            quote = char
        elif char == ";":
            return True
        index += 1
    return quote is not None


def parse_lines(content):
    queries = []
    for number, line in enumerate(content.splitlines(), 1):
        value = line.strip()
        if not value or value.startswith(("--", "#")):
            continue
        if value.endswith(";"):
            value = value[:-1].rstrip()
        if (not re.match(r"(?i)^SELECT\s", value) or _has_separator(value)
                or len(value.encode("utf-8")) > 4096):
            raise ValueError("line " + str(number) + ": one bounded SELECT is required")
        if len(queries) >= 200:
            raise ValueError("at most 200 SELECT queries per audit")
        queries.append((number, value))
    if not queries:
        raise ValueError("no SELECT queries provided")
    return queries


MAX_QUERY_FILE_BYTES = 1_048_576


def read_queries(path):
    path = Path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("SQL audit corpus must be a regular, non-symlinked file")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(path, flags)
    with os.fdopen(fd, "rb") as handle:
        opened = os.fstat(handle.fileno())
        if (not stat.S_ISREG(opened.st_mode) or
                (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino)):
            raise ValueError("SQL audit corpus changed while opening")
        raw = handle.read(MAX_QUERY_FILE_BYTES + 1)
        finished = os.fstat(handle.fileno())
        # A stable inode at open time is not sufficient if its bytes change
        # during the read. Never audit a mixture of corpus revisions.
        state = lambda value: (
            value.st_dev, value.st_ino, value.st_size,
            value.st_mtime_ns, value.st_ctime_ns,
        )
        if state(opened) != state(finished):
            raise ValueError("SQL audit corpus changed during read")
    if state(before) != state(path.lstat()):
        raise ValueError("SQL audit corpus changed during read")
    if len(raw) > MAX_QUERY_FILE_BYTES:
        raise ValueError("SQL audit corpus exceeds 1 MiB")
    return parse_lines(raw.decode("utf-8"))


def audit(binary, queries, run=subprocess.run, fail_fast=False, timeout=10):
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("audit timeout must be positive and finite")
    results = []
    with tempfile.TemporaryDirectory(prefix="rusthouse-sql-audit-") as database:
        for line, query in queries:
            try:
                # Keep noisy SQL CLI output out of process memory.
                with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
                    result = run(
                        [str(binary), "sql", query, "--database", database, "--explain"],
                        stdout=stdout, stderr=stderr, timeout=timeout, check=False,
                    )
                    status = result.returncode == 0
                    stderr.seek(0)
                    raw = stderr.read(500)
                    reason = "" if status else raw.decode("utf-8", "replace").strip()
                    # Preserve compatibility with injected test runners which
                    # return a stderr value instead of writing to the file.
                    if not status and not raw:
                        reported = getattr(result, "stderr", None)
                        if isinstance(reported, bytes):
                            reason = reported[:500].decode("utf-8", "replace").strip()
                        elif isinstance(reported, str):
                            reason = reported[:500].strip()
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
    parser.add_argument("--timeout", type=float, default=10,
                        help="Maximum seconds for each parser-only query")
    args = parser.parse_args()
    try:
        statements = read_queries(args.queries)
        result = audit(args.binary, statements, fail_fast=args.fail_fast,
                       timeout=args.timeout)
    except (OSError, UnicodeError, ValueError) as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2))
    return 0 if result["total"] == result["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

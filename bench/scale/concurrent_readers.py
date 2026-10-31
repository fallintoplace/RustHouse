"""Stress concurrent read-only CLI queries against the same frozen snapshot.

Uses a trusted reference file; this is not an official database benchmark.
"""
import argparse
import concurrent.futures
import json
import math
import re
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def _has_separator(statement):
    # Delimiters inside quoted SQL text do not separate statements.
    quote = None
    index = 0
    while index < len(statement):
        char = statement[index]
        if quote is not None:
            if char == "\\":
                index += 2
                continue
            if char == quote:
                if index + 1 < len(statement) and statement[index + 1] == quote:
                    index += 2
                    continue
                quote = None
        elif char in ("'", '"'):
            quote = char
        elif char == ";":
            return True
        index += 1
    return quote is not None


def read_reference(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError("reference must be a regular non-symlinked file")
    with path.open("rb") as stream:
        payload = stream.read(1024 * 1024 + 1)
    if len(payload) > 1024 * 1024:
        raise ValueError("reference exceeds 1 MiB")
    return payload


def run_readers(binary, part, sql, expected, *, clients=4, rounds=3,
                timeout=120, execute=None):
    if (type(clients) is not int or type(rounds) is not int
            or not 1 <= clients <= 64 or not 1 <= rounds <= 100
            or clients * rounds > 256):
        raise ValueError("clients*rounds must be within 1..256 and clients within 1..64")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    if not isinstance(expected, bytes) or len(expected) > 1024 * 1024:
        raise ValueError("expected result must be at most 1 MiB")
    if not isinstance(sql, str) or not sql.strip() or len(sql.encode("utf-8")) > 65536:
        raise ValueError("SQL must be nonempty and at most 64 KiB")
    statement = sql.strip()
    if statement.endswith(";"):
        statement = statement[:-1].rstrip()
    if not re.match(r"^(SELECT|WITH|EXPLAIN)\b", statement, re.IGNORECASE) or _has_separator(statement):
        raise ValueError("concurrent readers require one read-only SQL statement")
    run = execute or subprocess.run
    cmd = [str(binary), "sql", sql, "--part", str(part), "--format",
           "tsv-no-header", "--max-rows", "1000"]

    def one(_):
        start = time.perf_counter()
        try:
            if execute is None:
                # Stream to disk so a runaway result cannot fill worker memory.
                with tempfile.TemporaryFile() as sink:
                    result = run(cmd, stdout=sink, stderr=subprocess.DEVNULL,
                                 check=False, timeout=timeout)
                    sink.seek(0)
                    output = sink.read(len(expected) + 1)
            else:
                # Preserve the injected runner contract used by workload tests.
                result = run(cmd, capture_output=True, check=False, timeout=timeout)
                output = result.stdout
        except (OSError, subprocess.TimeoutExpired) as error:
            return {"ok": False, "seconds": time.perf_counter() - start, "error": str(error)}
        duration = time.perf_counter() - start
        if result.returncode:
            return {"ok": False, "seconds": duration, "error": "SQL process failed"}
        matched = output == expected
        return {"ok": matched, "seconds": duration,
                "error": None if matched else "result differs from reference"}

    started = time.perf_counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=clients) as pool:
        results = list(pool.map(one, range(clients * rounds)))
    elapsed = time.perf_counter() - started
    success = [r["seconds"] for r in results if r["ok"]]
    ordered = sorted(success)
    def percentile(percent):
        return ordered[math.ceil(len(ordered) * percent / 100) - 1] if ordered else None

    return {"workload": "verified-offline-concurrent-reads",
            "clients": clients, "rounds": rounds, "planned": len(results),
            "verified": len(success), "all_verified": len(success) == len(results),
            "elapsed_seconds": elapsed,
            "verified_queries_per_second": len(success) / elapsed if elapsed > 0 else None,
            "median_query_seconds": statistics.median(success) if success else None,
            "p95_query_seconds": percentile(95),
            "p99_query_seconds": percentile(99),
            "results": results}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--sql", required=True)
    parser.add_argument("--expected-file", type=Path, required=True)
    parser.add_argument("--clients", type=int, default=4)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        expected = read_reference(args.expected_file)
        report = run_readers(args.binary, args.part, args.sql,
                             expected, clients=args.clients,
                             rounds=args.rounds, timeout=args.timeout)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, ValueError) as error:
        parser.exit(1, f"concurrent reads failed: {error}\n")
    print(rendered, end="")
    return 0 if report["all_verified"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Run Analytics SQL against the RustHouse CLI or HTTP server."""
import argparse
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, latency_summary


DEFAULT_QUERIES = Path(__file__).with_name("queries.sql")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """Do not forward benchmark SQL bodies to an unexpected endpoint."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


HTTP_OPENER = urllib.request.build_opener(NoRedirect())


def load_queries(path):
    lines = path.read_text(encoding="utf-8").splitlines()
    return [line.strip().rstrip(";") for line in lines if line.strip() and not line.lstrip().startswith("--")]


def select_indices(spec, count):
    if not spec:
        return list(range(count))
    indices = set()
    for fragment in spec.split(","):
        pair = fragment.split("-", 1)
        first = int(pair[0])
        last = int(pair[1]) if len(pair) == 2 else first
        if not (1 <= first <= last <= count):
            raise ValueError("query numbers must be within 1..%d" % count)
        indices.update(range(first - 1, last))
    return sorted(indices)


def execute_cli(binary, database, sql, timeout):
    return cli_sql(binary, database, sql, timeout)


def http_target(endpoint):
    parsed = urllib.parse.urlsplit(endpoint)
    if parsed.scheme not in ("http", "https") or not parsed.netloc:
        raise ValueError("--url requires an HTTP(S) URL")
    params = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
    params.append(("format", "tsv-no-header"))
    return urllib.parse.urlunsplit(
        (parsed.scheme, parsed.netloc, parsed.path or "/", urllib.parse.urlencode(params), "")
    )


def execute_http(endpoint, sql, timeout):
    request = urllib.request.Request(http_target(endpoint), sql.encode("utf-8"), method="POST")
    try:
        with HTTP_OPENER.open(request, timeout=timeout) as response:
            return response.read()
    except urllib.error.HTTPError as error:
        raise RuntimeError("HTTP %d: %s" % (error.code, error.read(2000).decode("utf-8", "replace"))) from error


def verify_input(execute, sql, expected, timeout):
    output = execute(sql, timeout)
    actual = output.removesuffix(b"\r\n").removesuffix(b"\n")
    reference = expected.encode("utf-8").removesuffix(b"\r\n").removesuffix(b"\n")
    if actual != reference:
        raise ValueError("preflight result differs from the expected output")


def summarize_times(timings):
    return latency_summary(timings)


def reference_failure(expected_path, output):
    if not expected_path.is_file():
        return "missing_reference", str(expected_path)
    if output != expected_path.read_bytes():
        return "mismatch", "query output differs from expected TSV"
    return None


def run_benchmark(queries, indices, runs, timeout, execute, reference_dir=None, warmups=0):
    results = []
    for index in indices:
        sql = queries[index]
        expected_path = None if reference_dir is None else reference_dir / ("q%02d.tsv" % (index + 1))
        timings = []
        status = "unverified" if expected_path is None else "passed"
        failure = None
        for iteration in range(-warmups, runs):
            start = time.perf_counter_ns() if iteration >= 0 else None
            try:
                output = execute(sql, timeout)
            except (OSError, RuntimeError, subprocess.TimeoutExpired, urllib.error.URLError) as error:
                status, failure = "error", str(error)[:2000]
                break
            # Reference file I/O and comparisons are outside the timed query.
            finished = time.perf_counter_ns() if start is not None else None
            if expected_path is not None:
                invalid = reference_failure(expected_path, output)
                if invalid is not None:
                    status, failure = invalid
                    break
            if finished is not None:
                elapsed = (finished - start) / 1_000_000_000
                timings.append(round(elapsed, 9))
        if status not in ("passed", "unverified"):
            timings = []
        results.append({
            "query": index + 1,
            "status": status,
            "seconds": timings,
            "latency": summarize_times(timings),
            "error": failure,
        })
    return results


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--binary", type=Path, help="Built RustHouse CLI binary")
    mode.add_argument("--url", help="RustHouse HTTP server root URL")
    parser.add_argument("--database", type=Path, help="Required with --binary")
    parser.add_argument("--queries", type=Path, default=DEFAULT_QUERIES)
    parser.add_argument("--only", help="Query numbers, e.g. 1,3-5")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--reference-dir", type=Path, help="Exact q01.tsv ... q43.tsv expected outputs")
    parser.add_argument("--preflight-sql", help="Optional query to validate the input data")
    parser.add_argument("--preflight-expected", help="Expected UTF-8 output from --preflight-sql")
    parser.add_argument("--output", type=Path, help="JSON result file")
    args = parser.parse_args(argv)
    if args.binary and not args.database:
        parser.error("--database is required with --binary")
    if (args.preflight_sql is None) != (args.preflight_expected is None):
        parser.error("--preflight-sql and --preflight-expected must be used together")
    if args.preflight_sql is not None and not args.preflight_sql.strip():
        parser.error("--preflight-sql cannot be empty")
    if args.runs < 1 or args.timeout <= 0 or args.warmups < 0:
        parser.error("--runs and --timeout must be positive; --warmups must be nonnegative")
    queries = load_queries(args.queries)
    if args.queries.resolve() == DEFAULT_QUERIES.resolve() and len(queries) != 43:
        parser.error("pinned workload must contain exactly 43 queries")
    if not queries:
        parser.error("query file is empty")
    try:
        indices = select_indices(args.only, len(queries))
    except ValueError as error:
        parser.error(str(error))
    if args.binary:
        execute = lambda sql, timeout: execute_cli(args.binary, args.database, sql, timeout)
        engine = "rusthouse-cli"
    else:
        execute = lambda sql, timeout: execute_http(args.url, sql, timeout)
        engine = "rusthouse-http"
    if args.preflight_sql is not None:
        try:
            verify_input(execute, args.preflight_sql, args.preflight_expected, args.timeout)
        except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired, urllib.error.URLError) as error:
            parser.exit(1, "input validation failed: %s\n" % error)
    results = run_benchmark(
        queries, indices, args.runs, args.timeout, execute, args.reference_dir, args.warmups
    )
    report = {
        "engine": engine,
        "runs": args.runs,
        "warmups": args.warmups,
        "reference_checked": args.reference_dir is not None,
        "results": results,
    }
    emit_json(report, args.output)
    return int(any(item["status"] not in ("passed", "unverified") for item in results))


if __name__ == "__main__":
    raise SystemExit(main())

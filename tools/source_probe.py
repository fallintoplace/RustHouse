#!/usr/bin/env python3
"""Validate the configured RustHouse HTTP source, not only process liveness."""
import argparse
import json
import math
import re
import sys
from urllib.parse import quote, urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener


class NoRedirect(HTTPRedirectHandler):
    """Prevent loopback-only probes from following redirects to remote hosts."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


NO_REDIRECT_OPENER = build_opener(NoRedirect())


def _unique_object_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate source probe JSON field: {key}")
        result[key] = value
    return result


def verify_origin(value):
    parsed = urlsplit(value)
    if (parsed.scheme != "http" or parsed.hostname not in ("localhost", "127.0.0.1")
            or parsed.path not in ("", "/") or parsed.query or parsed.fragment
            or parsed.username or parsed.password):
        raise ValueError("source probe accepts loopback HTTP origins only")
    return value.rstrip("/")


def read_json(url, timeout):
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or not 0 < timeout <= 300:
        raise ValueError("timeout must be finite and in (0, 300] seconds")
    with NO_REDIRECT_OPENER.open(url, timeout=timeout) as reply:
        payload = reply.read(1_048_577)
    if len(payload) > 1_048_576:
        raise ValueError("response exceeds 1 MiB")
    return json.loads(payload, object_pairs_hook=_unique_object_pairs)


def scan_rows(base, table, timeout):
    """Execute a real bounded SELECT to check data-path readiness."""
    query = f"SELECT count(*) AS rows_checked FROM {table}".encode("ascii")
    request = Request(base + "/", data=query,
                      headers={"Content-Type": "text/plain; charset=utf-8"},
                      method="POST")
    with NO_REDIRECT_OPENER.open(request, timeout=timeout) as response:
        body = response.read(129)
    if len(body) > 128 or not re.fullmatch(rb"rows_checked\n[0-9]{1,20}\n", body):
        raise ValueError("readiness scan returned invalid SQL count output")
    count = int(body.splitlines()[1])
    if count > (1 << 64) - 1:
        raise ValueError("readiness scan count exceeds UInt64")
    return count


def probe(base, expected_source, table=None, timeout=5, expected_version=None, verify_scan=False):
    base = verify_origin(base)
    if expected_source not in ("generated", "part", "database"):
        raise ValueError("expected_source must be generated, part, or database")
    if expected_version is not None and (
            not isinstance(expected_version, str) or not expected_version):
        raise ValueError("expected_version must be a nonempty string")
    if table is not None:
        if expected_source != "database":
            raise ValueError("--table requires database mode")
        if not isinstance(table, str) or not re.fullmatch(
                r"[A-Za-z_][A-Za-z0-9_]{0,127}", table):
            raise ValueError("invalid table")
    if verify_scan and table is None:
        raise ValueError("--verify-scan requires --table")
    data = read_json(base + "/capabilities", timeout)
    if not isinstance(data, dict):
        raise ValueError("invalid capabilities response")
    if data.get("read_only") is not True:
        raise ValueError("server is not marked read-only")
    if data.get("source") != expected_source:
        raise ValueError("expected source " + expected_source + ", got " + str(data.get("source")))
    if expected_version is not None:
        if data.get("version") != expected_version:
            raise ValueError("expected RustHouse version " + expected_version +
                             ", got " + str(data.get("version")))
    if table is not None:
        schema = read_json(base + "/schema?table=" + quote(table, safe=""), timeout)
        if not isinstance(schema, dict) or not isinstance(schema.get("columns"), list):
            raise ValueError("missing schema columns")
        columns = schema["columns"]
        if not columns or len(columns) > 128:
            raise ValueError("schema must contain 1..128 columns")
        names = set()
        for column in columns:
            if (not isinstance(column, dict) or not isinstance(column.get("name"), str)
                    or not isinstance(column.get("type"), str)
                    or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,127}", column["name"])
                    or not column["type"].strip() or column["name"] in names):
                raise ValueError("invalid schema column")
            names.add(column["name"])
    if verify_scan:
        scan_rows(base, table, timeout)
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8123")
    parser.add_argument("--timeout", type=float, default=5)
    parser.add_argument("--source", choices=("generated", "part", "database"), required=True)
    parser.add_argument("--table")
    parser.add_argument("--expected-version", help="Reject unexpected server versions")
    parser.add_argument("--verify-scan", action="store_true",
                        help="Require a real SELECT count(*) to succeed")
    args = parser.parse_args()
    try:
        info = probe(verify_origin(args.url), args.source, args.table,
                     expected_version=args.expected_version, timeout=args.timeout,
                     verify_scan=args.verify_scan)
    except Exception as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print("PASS: RustHouse " + str(info.get("version")) + " source=" + args.source)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

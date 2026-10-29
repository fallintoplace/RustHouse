#!/usr/bin/env python3
"""Validate the configured RustHouse HTTP source, not only process liveness."""
import argparse
import json
import re
import sys
from urllib.parse import quote, urlsplit
from urllib.request import urlopen


def verify_origin(value):
    parsed = urlsplit(value)
    if (parsed.scheme != "http" or parsed.hostname not in ("localhost", "127.0.0.1")
            or parsed.path not in ("", "/") or parsed.query or parsed.fragment
            or parsed.username or parsed.password):
        raise ValueError("source probe accepts loopback HTTP origins only")
    return value.rstrip("/")


def read_json(url, timeout):
    with urlopen(url, timeout=timeout) as reply:
        payload = reply.read(1_048_577)
    if len(payload) > 1_048_576:
        raise ValueError("response exceeds 1 MiB")
    return json.loads(payload)


def probe(base, expected_source, table=None, timeout=5, expected_version=None):
    data = read_json(base + "/capabilities", timeout)
    if data.get("read_only") is not True:
        raise ValueError("server is not marked read-only")
    if data.get("source") != expected_source:
        raise ValueError("expected source " + expected_source + ", got " + str(data.get("source")))
    if expected_version is not None:
        if not isinstance(expected_version, str) or not expected_version:
            raise ValueError("expected_version must be a nonempty string")
        if data.get("version") != expected_version:
            raise ValueError("expected RustHouse version " + expected_version +
                             ", got " + str(data.get("version")))
    if table is not None:
        if expected_source != "database":
            raise ValueError("--table requires database mode")
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,127}", table):
            raise ValueError("invalid table")
        schema = read_json(base + "/schema?table=" + quote(table, safe=""), timeout)
        if not isinstance(schema, dict) or not isinstance(schema.get("columns"), list):
            raise ValueError("missing schema columns")
        columns = schema["columns"]
        if not columns:
            raise ValueError("schema has no columns")
        names = set()
        for column in columns:
            if (not isinstance(column, dict) or not isinstance(column.get("name"), str)
                    or not isinstance(column.get("type"), str)
                    or not column["name"].strip() or not column["type"].strip()
                    or column["name"] in names):
                raise ValueError("invalid schema column")
            names.add(column["name"])
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8123")
    parser.add_argument("--source", choices=("generated", "part", "database"), required=True)
    parser.add_argument("--table")
    parser.add_argument("--expected-version", help="Reject unexpected server versions")
    args = parser.parse_args()
    try:
        info = probe(verify_origin(args.url), args.source, args.table,
                     expected_version=args.expected_version)
    except Exception as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print("PASS: RustHouse " + str(info.get("version")) + " source=" + args.source)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

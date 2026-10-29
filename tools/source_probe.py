#!/usr/bin/env python3
"""Validate the configured RustHouse HTTP source, not only process liveness."""
import argparse
import json
import re
import sys
from urllib.parse import quote, urlsplit
from urllib.request import urlopen


MAX_RESPONSE_BYTES = 1_048_576


def verify_origin(value):
    parsed = urlsplit(value)
    if (parsed.scheme != "http" or parsed.hostname not in ("localhost", "127.0.0.1")
            or parsed.path not in ("", "/") or parsed.query or parsed.fragment
            or parsed.username or parsed.password):
        raise ValueError("source probe accepts loopback HTTP origins only")
    return value.rstrip("/")


def read_json(url, timeout):
    with urlopen(url, timeout=timeout) as reply:
        payload = reply.read(MAX_RESPONSE_BYTES + 1)
    if len(payload) > MAX_RESPONSE_BYTES:
        raise ValueError("response exceeds 1 MiB")
    return json.loads(payload)


def probe(base, expected_source, table=None, timeout=5):
    data = read_json(base + "/capabilities", timeout)
    if data.get("read_only") is not True:
        raise ValueError("server is not marked read-only")
    if data.get("source") != expected_source:
        raise ValueError("expected source " + expected_source + ", got " + str(data.get("source")))
    if table is not None:
        if expected_source != "database":
            raise ValueError("--table requires database mode")
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,127}", table):
            raise ValueError("invalid table")
        schema = read_json(base + "/schema?table=" + quote(table, safe=""), timeout)
        if not isinstance(schema.get("columns"), list):
            raise ValueError("missing schema columns")
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8123")
    parser.add_argument("--source", choices=("generated", "part", "database"), required=True)
    parser.add_argument("--table")
    args = parser.parse_args()
    try:
        info = probe(verify_origin(args.url), args.source, args.table)
    except Exception as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print("PASS: RustHouse " + str(info.get("version")) + " source=" + args.source)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Compare ordered RustHouse table schema between two loopback servers."""
import argparse
import json
import re
import sys
from urllib.parse import quote, urlsplit
from urllib.request import urlopen


MAX_SCHEMA_BYTES = 1_048_576


def validate_url(origin):
    url = urlsplit(origin)
    if (url.scheme != "http" or url.hostname not in ("localhost", "127.0.0.1")
            or url.path not in ("", "/") or url.query or url.fragment
            or url.username or url.password):
        raise ValueError("only loopback HTTP origins supported")
    return origin.rstrip("/")


def columns(payload):
    if not isinstance(payload, dict):
        raise ValueError("schema response must be an object")
    result = payload.get("columns")
    if not isinstance(result, list):
        raise ValueError("response has no columns list")
    output = []
    seen = set()
    for col in result:
        if not isinstance(col, dict) or not isinstance(col.get("name"), str) or not isinstance(col.get("type"), str):
            raise ValueError("malformed column")
        if not col["name"] or not col["type"] or col["name"] in seen:
            raise ValueError("empty or duplicate schema column")
        seen.add(col["name"])
        output.append((col["name"], col["type"]))
    return output


def fetch(base, table, timeout=5):
    url = base + "/schema?table=" + quote(table, safe="")
    with urlopen(url, timeout=timeout) as response:
        body = response.read(MAX_SCHEMA_BYTES + 1)
    if len(body) > MAX_SCHEMA_BYTES:
        raise ValueError("schema response too large")
    return columns(json.loads(body))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--left", required=True)
    parser.add_argument("--right", required=True)
    parser.add_argument("--table", required=True)
    args = parser.parse_args()
    try:
        if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]{0,127}", args.table):
            raise ValueError("invalid table name")
        left = fetch(validate_url(args.left), args.table)
        right = fetch(validate_url(args.right), args.table)
        if left != right:
            print("MISMATCH: ordered column names/types differ", file=sys.stderr)
            print("left: " + repr(left), file=sys.stderr)
            print("right: " + repr(right), file=sys.stderr)
            return 1
    except Exception as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print("PASS: identical ordered schema for " + args.table)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

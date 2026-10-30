#!/usr/bin/env python3
"""Compare ordered RustHouse table schema between two loopback servers."""
import argparse
import json
import re
import sys
from urllib.parse import quote, urlsplit
from urllib.request import HTTPRedirectHandler, build_opener


class NoRedirect(HTTPRedirectHandler):
    """A loopback endpoint must not redirect requests to other hosts."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


NO_REDIRECT_OPENER = build_opener(NoRedirect())


def _unique_object_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate schema JSON field: {key}")
        result[key] = value
    return result


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
    if not isinstance(result, list) or not result:
        raise ValueError("response has no nonempty columns list")
    output = []
    seen = set()
    for col in result:
        if not isinstance(col, dict) or not isinstance(col.get("name"), str) or not isinstance(col.get("type"), str):
            raise ValueError("malformed column")
        if not col["name"].strip() or not col["type"].strip() or col["name"] in seen:
            raise ValueError("empty or duplicate schema column")
        seen.add(col["name"])
        output.append((col["name"], col["type"]))
    return output


def compatible_columns(left, right, *, allow_additive=False):
    if left == right:
        return True
    return bool(allow_additive and len(right) >= len(left)
                and right[:len(left)] == left)


def fetch(base, table, timeout=5):
    url = base + "/schema?table=" + quote(table, safe="")
    with NO_REDIRECT_OPENER.open(url, timeout=timeout) as response:
        body = response.read(1_048_577)
    if len(body) > 1_048_576:
        raise ValueError("schema response too large")
    return columns(json.loads(body, object_pairs_hook=_unique_object_pairs))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--left", required=True)
    parser.add_argument("--right", required=True)
    parser.add_argument("--table", required=True)
    parser.add_argument("--allow-additive", action="store_true")
    args = parser.parse_args()
    try:
        if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]{0,127}", args.table):
            raise ValueError("invalid table name")
        left = fetch(validate_url(args.left), args.table)
        right = fetch(validate_url(args.right), args.table)
        if not compatible_columns(left, right, allow_additive=args.allow_additive):
            print("MISMATCH: ordered column names/types differ", file=sys.stderr)
            print("left: " + repr(left), file=sys.stderr)
            print("right: " + repr(right), file=sys.stderr)
            return 1
    except Exception as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    status = ("append-only compatible" if left != right else "identical ordered")
    print("PASS: " + status + " schema for " + args.table)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

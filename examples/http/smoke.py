#!/usr/bin/env python3
"""Check read-only RustHouse endpoints without touching stored data."""
import argparse
import ipaddress
import json
import sys
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener


class NoRedirect(HTTPRedirectHandler):
    """Do not forward RustHouse diagnostics to redirect destinations."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


# Experimental unauthenticated probes must not use host HTTP proxy settings.
urlopen = build_opener(ProxyHandler({}), NoRedirect()).open


def origin(value, allow_private=False):
    url = urlsplit(value)
    if (url.scheme != "http" or not url.hostname or url.username or
            url.password or url.path not in ("", "/") or url.query or url.fragment):
        raise ValueError("expected an HTTP origin")
    if url.hostname not in ("localhost", "127.0.0.1"):
        if not allow_private:
            raise ValueError("non-loopback requires explicit --allow-private")
        try:
            address = ipaddress.IPv4Address(url.hostname)
        except ipaddress.AddressValueError as exc:
            raise ValueError("--allow-private requires numeric private IPv4") from exc
        first, second, _, _ = address.packed
        if not (first == 10 or first == 172 and 16 <= second <= 31
                or first == 192 and second == 168
                or first == 100 and 64 <= second <= 127):
            raise ValueError("--allow-private requires numeric private IPv4")
    return value.rstrip("/")


def request(host, path, sql=None, timeout=5):
    req = Request(host + path, data=sql.encode() if sql is not None else None,
                  method="POST" if sql is not None else "GET")
    with urlopen(req, timeout=timeout) as response:
        body = response.read(1024 * 1024 + 1)
    if len(body) > 1024 * 1024:
        raise ValueError("smoke response exceeded 1 MiB")
    return body.decode("utf-8")


def check(host, timeout=5):
    if request(host, "/ping", timeout=timeout) != "Ok.\n":
        raise ValueError("invalid ping response")
    capabilities = json.loads(request(host, "/capabilities", timeout=timeout))
    if capabilities.get("read_only") is not True:
        raise ValueError("server is not read-only")
    rows = json.loads(request(host, "/?format=json",
                              "SELECT number FROM numbers(3)", timeout=timeout))
    if rows != [{"number": "0"}, {"number": "1"}, {"number": "2"}]:
        raise ValueError("incorrect generated query results")
    if "Logical plan" not in request(host, "/explain", "SELECT count(*) FROM numbers(3)", timeout):
        raise ValueError("missing logical EXPLAIN plan")
    return capabilities


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8123")
    parser.add_argument("--timeout", type=float, default=5)
    parser.add_argument("--allow-private", action="store_true")
    args = parser.parse_args()
    try:
        if not 0 < args.timeout <= 120:
            raise ValueError("--timeout must be between 0 and 120 seconds")
        info = check(origin(args.url, args.allow_private), args.timeout)
    except Exception as error:
        print("FAIL: " + str(error), file=sys.stderr)
        return 1
    print("PASS: RustHouse " + str(info.get("version")) + ", HTTP and exact SELECT")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

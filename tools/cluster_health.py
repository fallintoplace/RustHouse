#!/usr/bin/env python3
"""Check RustHouse peer reachability from /cluster/health (not data freshness)."""
import argparse
import json
import sys
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, build_opener
from urllib.error import HTTPError


class NoRedirect(HTTPRedirectHandler):
    """A loopback endpoint must not redirect requests to other hosts."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


NO_REDIRECT_OPENER = build_opener(NoRedirect())


def validate(payload):
    if not isinstance(payload, dict):
        raise ValueError("expected JSON object")
    total, healthy, peers = payload.get("total"), payload.get("healthy"), payload.get("peers")
    if (type(total) is not int or type(healthy) is not int or
            not isinstance(peers, list) or total != len(peers) or
            not 0 <= healthy <= total):
        raise ValueError("invalid peer-health metadata")
    actual = 0
    for peer in peers:
        if (not isinstance(peer, dict) or not isinstance(peer.get("address"), str) or
                type(peer.get("healthy")) is not bool):
            raise ValueError("invalid peer detail")
        actual += peer["healthy"]
    if actual != healthy:
        raise ValueError("peer count inconsistent with details")
    return total == healthy


def inspect(origin, timeout=5):
    url = urlsplit(origin)
    if (url.scheme != "http" or url.hostname not in ("localhost", "127.0.0.1")
            or url.path not in ("", "/") or url.query or url.fragment
            or url.username or url.password):
        raise ValueError("only loopback HTTP origins are accepted")
    try:
        with NO_REDIRECT_OPENER.open(origin.rstrip("/") + "/cluster/health", timeout=timeout) as response:
            raw = response.read(1_048_577)
    except HTTPError as exc:
        if exc.code != 503:
            raise
        raw = exc.read(1_048_577)
    if len(raw) > 1_048_576:
        raise ValueError("peer-health response too large")
    payload = json.loads(raw)
    return validate(payload), payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8123")
    args = parser.parse_args()
    try:
        all_healthy, info = inspect(args.url)
    except Exception as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print(str(info["healthy"]) + "/" + str(info["total"]) + " RustHouse peers healthy")
    return 0 if all_healthy else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Check RustHouse peer reachability from /cluster/health (not data freshness)."""
import argparse
import json
import ipaddress
import math
import sys
from loopback_origin import require_loopback_http_origin
from urllib.request import HTTPRedirectHandler, build_opener, ProxyHandler
from urllib.error import HTTPError


class NoRedirect(HTTPRedirectHandler):
    """A loopback endpoint must not redirect requests to other hosts."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


NO_REDIRECT_OPENER = build_opener(ProxyHandler({}), NoRedirect())
MAX_PEERS = 32


def _reject_nonfinite_json(value):
    # Python's json.loads accepts NaN and Infinity unless explicitly blocked.
    raise ValueError(f"nonfinite JSON constant: {value}")


def _finite_json_float(value):
    # A legal JSON exponent such as 1e9999 also becomes infinity in Python.
    number = float(value)
    if not math.isfinite(number):
        raise ValueError(f"nonfinite JSON number: {value}")
    return number


def _unique_object_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate peer-health JSON field: {key}")
        result[key] = value
    return result


def valid_peer_address(value):
    """Match the coordinator's canonical numeric IPv4 peer address rules."""
    if not isinstance(value, str) or value.count(":") != 1:
        return False
    host, port = value.split(":")
    if not 1 <= len(port) <= 5 or not port.isascii() or not port.isdecimal():
        return False
    number = int(port)
    if not 1 <= number <= 65535:
        return False
    try:
        address = ipaddress.IPv4Address(host)
    except ipaddress.AddressValueError:
        return False
    if value != f"{address}:{number}":
        return False
    first, second, _, _ = address.packed
    return (str(address) == "127.0.0.1" or first == 10 or
            (first == 172 and 16 <= second <= 31) or
            (first == 192 and second == 168) or
            (first == 100 and 64 <= second <= 127))


def validate(payload, *, min_healthy=None, expected_peers=None):
    if not isinstance(payload, dict):
        raise ValueError("expected JSON object")
    total, healthy, peers = payload.get("total"), payload.get("healthy"), payload.get("peers")
    if (type(total) is not int or type(healthy) is not int or
            not isinstance(peers, list) or total != len(peers) or
            not 0 <= healthy <= total or total > MAX_PEERS):
        raise ValueError("invalid peer-health metadata")
    actual = 0
    seen = set()
    for peer in peers:
        if (not isinstance(peer, dict) or not isinstance(peer.get("address"), str) or
                not valid_peer_address(peer["address"]) or type(peer.get("healthy")) is not bool):
            raise ValueError("invalid peer detail")
        if peer["address"] in seen:
            raise ValueError("duplicate peer address")
        seen.add(peer["address"])
        actual += peer["healthy"]
    if actual != healthy:
        raise ValueError("peer count inconsistent with details")
    if expected_peers is not None:
        if (not isinstance(expected_peers, (list, tuple)) or not expected_peers or
                any(not valid_peer_address(peer) for peer in expected_peers) or
                len(set(expected_peers)) != len(expected_peers) or
                set(expected_peers) != seen):
            raise ValueError("unexpected cluster membership")
    if min_healthy is not None:
        if type(min_healthy) is not int or not 1 <= min_healthy <= total:
            raise ValueError("min_healthy must be within the reported peer count")
        return healthy >= min_healthy
    # Zero configured peers cannot constitute a healthy cluster.
    return total > 0 and total == healthy


def inspect(origin, timeout=5, min_healthy=None, expected_peers=None):
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or not 0 < timeout <= 300:
        raise ValueError("timeout must be finite and in (0, 300] seconds")
    origin = require_loopback_http_origin(origin)
    unavailable = False
    try:
        with NO_REDIRECT_OPENER.open(origin + "/cluster/health", timeout=timeout) as response:
            raw = response.read(1_048_577)
    except HTTPError as exc:
        try:
            if exc.code != 503:
                raise
            unavailable = True
            raw = exc.read(1_048_577)
        finally:
            exc.close()
    if len(raw) > 1_048_576:
        raise ValueError("peer-health response too large")
    payload = json.loads(raw, object_pairs_hook=_unique_object_pairs,
                         parse_constant=_reject_nonfinite_json, parse_float=_finite_json_float)
    healthy = validate(payload, min_healthy=min_healthy, expected_peers=expected_peers)
    return healthy and not unavailable, payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8123")
    parser.add_argument("--timeout", type=float, default=5)
    parser.add_argument("--min-healthy", type=int,
                        help="Require at least this many reachable peers")
    parser.add_argument("--expected-peers",
                        help="Comma-separated exact peer addresses for deployment verification")
    args = parser.parse_args()
    try:
        expected = args.expected_peers.split(",") if args.expected_peers is not None else None
        all_healthy, info = inspect(args.url, min_healthy=args.min_healthy,
                                    expected_peers=expected, timeout=args.timeout)
    except Exception as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print(str(info["healthy"]) + "/" + str(info["total"]) + " RustHouse peers healthy")
    return 0 if all_healthy else 1


if __name__ == "__main__":
    raise SystemExit(main())

"""Canonical loopback-only HTTP origin validation for diagnostic clients."""
import re

# Match the raw authority rather than only urlsplit().hostname. urlsplit
# normalizes control characters, and accepts ports that urllib cannot use.
_ORIGIN = re.compile(
    r"http://(?:localhost|127\.0\.0\.1)(?::([1-9][0-9]{0,4}))?/?\Z",
    re.IGNORECASE,
)


def require_loopback_http_origin(origin):
    if not isinstance(origin, str):
        raise ValueError("only canonical loopback HTTP origins are accepted")
    match = _ORIGIN.fullmatch(origin)
    if match is None:
        raise ValueError("only canonical loopback HTTP origins are accepted")
    port = match.group(1)
    if port is not None and int(port) > 65535:
        raise ValueError("loopback HTTP port must be between 1 and 65535")
    return origin.rstrip("/")

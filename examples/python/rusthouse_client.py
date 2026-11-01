"""Dependency-free, read-only RustHouse HTTP client for Python 3.9+.

RustHouse's HTTP server is experimental and lacks TLS and authentication.
Non-loopback use is rejected by default. Never expose the server publicly.
JSON numbers are intentionally represented as strings by RustHouse.
"""
from __future__ import annotations

import json
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener


class NoRedirect(HTTPRedirectHandler):
    """Never forward unauthenticated RustHouse requests to a redirect target."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


# Do not expose local unauthenticated traffic to environment HTTP proxies.
urlopen = build_opener(ProxyHandler({}), NoRedirect()).open


class RustHouseError(Exception):
    """A transport, protocol, or SQL error returned by RustHouse."""


class RustHouseHTTPError(RustHouseError):
    def __init__(self, status: int, body: str):
        self.status = status
        self.body = body
        super().__init__(f"RustHouse HTTP {status}: {body[:200].strip()}")


class RustHouseClient:
    def __init__(self, base_url: str = "http://127.0.0.1:8123", *,
                 timeout: float = 5.0, allow_private_network: bool = False):
        parsed = urlsplit(base_url)
        if (parsed.scheme != "http" or not parsed.hostname or parsed.username
                or parsed.password or parsed.path not in ("", "/")
                or parsed.query or parsed.fragment or not 0 < timeout <= 120):
            raise ValueError("expected an HTTP origin and timeout in (0, 120]")
        if parsed.hostname not in {"localhost", "127.0.0.1"} and not allow_private_network:
            raise ValueError("non-loopback hosts require allow_private_network=True")
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    def _request(self, path: str, sql: str | None = None) -> bytes:
        if sql is not None and (not sql or len(sql.encode("utf-8")) > 4096):
            raise ValueError("SQL must contain 1..4096 UTF-8 bytes")
        request = Request(
            self.base_url + path,
            data=sql.encode("utf-8") if sql is not None else None,
            headers={"Content-Type": "text/plain; charset=utf-8"} if sql is not None else {},
            method="POST" if sql is not None else "GET",
        )
        try:
            with urlopen(request, timeout=self.timeout) as response:
                body = response.read(16 * 1024 * 1024 + 1)
                if len(body) > 16 * 1024 * 1024:
                    raise RustHouseError("HTTP response exceeds 16 MiB client limit")
                return body
        except HTTPError as exc:
            message = exc.read(4096).decode("utf-8", "replace")
            raise RustHouseHTTPError(exc.code, message) from exc
        except (URLError, TimeoutError, OSError) as exc:
            raise RustHouseError(f"cannot reach RustHouse: {exc}") from exc

    def _json(self, path: str, sql: str | None = None):
        raw = self._request(path, sql)
        if len(raw) > 16 * 1024 * 1024:
            raise RustHouseError("HTTP response exceeds 16 MiB client limit")
        try:
            return json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeError) as exc:
            raise RustHouseError("invalid JSON from RustHouse") from exc

    def capabilities(self) -> dict:
        return self._json("/capabilities")

    def tables(self):
        return self._json("/tables")

    def schema(self, table: str | None = None) -> dict:
        if table is None:
            return self._json("/schema")
        if not table or len(table.encode("utf-8")) > 128:
            raise ValueError("invalid table name")
        return self._json("/schema?table=" + quote(table, safe=""))

    def query(self, sql: str) -> list:
        """Execute read-only SELECT and retain server-encoded string numerics."""
        rows = self._json("/?format=json", sql)
        if not isinstance(rows, list):
            raise RustHouseError("expected a JSON array of rows")
        return rows

    def explain(self, sql: str) -> str:
        raw = self._request("/explain", sql)
        return raw.decode("utf-8")

"""Compare persisted SQL results through the CLI and read-only HTTP server.

This is a functional parity check, not a benchmark or a wire-protocol test.
It creates a disposable single-writer database and listens on loopback only.
"""
import argparse
import json
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """Do not follow HTTP redirects to an unverified parity server."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


NO_REDIRECT_OPENER = urllib.request.build_opener(NoRedirect())


QUERIES = (
    "SELECT id, name FROM events ORDER BY id",
    "SELECT count(*) FROM events",
    "SELECT id FROM events WHERE id >= 2 ORDER BY id",
    "SELECT name FROM events WHERE name IS NULL",
)


def cli(binary, directory, statement):
    result = subprocess.run(
        [str(binary), "sql", statement, "--database", str(directory)],
        capture_output=True,
        timeout=30,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(
            f"CLI failed for {statement}: {result.stderr.decode('utf-8', 'replace')[-500:]}"
        )
    return result.stdout


def unused_loopback_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(port, statement):
    url = f"http://127.0.0.1:{port}/?format=tsv"
    req = urllib.request.Request(url, data=statement.encode("utf-8"), method="POST")
    with NO_REDIRECT_OPENER.open(req, timeout=10) as response:
        if response.status != 200:
            raise RuntimeError(f"HTTP SQL returned {response.status}")
        # A parity failure should not consume unbounded memory.
        payload = response.read(4 * 1024 * 1024 + 1)
        if len(payload) > 4 * 1024 * 1024:
            raise ValueError("HTTP parity SQL response exceeds 4 MiB")
        return payload


def verify_server_identity(port):
    """Require the intended RustHouse database-mode read-only endpoint."""
    with NO_REDIRECT_OPENER.open(f"http://127.0.0.1:{port}/capabilities", timeout=5) as response:
        body = response.read(1_048_577)
    if len(body) > 1_048_576:
        raise ValueError("HTTP parity capabilities response exceeds 1 MiB")
    data = json.loads(body)
    if (not isinstance(data, dict) or data.get("name") != "RustHouse"
            or data.get("source") != "database" or data.get("read_only") is not True):
        raise ValueError("HTTP parity server capabilities do not match RustHouse database mode")
    return data


def wait_ready(process, port):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("HTTP server exited before becoming ready")
        try:
            with NO_REDIRECT_OPENER.open(f"http://127.0.0.1:{port}/ping", timeout=0.4) as response:
                if response.status == 200:
                    return
        except (OSError, urllib.error.URLError):
            time.sleep(0.1)
    raise RuntimeError("HTTP server was not ready within 10 seconds")


def check_parity(binary, server):
    with tempfile.TemporaryDirectory(prefix="rusthouse-cli-http-parity-") as temporary:
        directory = Path(temporary) / "database"
        cli(binary, directory, "CREATE TABLE events (id UInt64, name Nullable(String))")
        cli(binary, directory, "INSERT INTO events VALUES (1, 'Ada'), (2, NULL), (3, 'Bob')")

        port = unused_loopback_port()
        process = subprocess.Popen(
            [str(server), "--database", str(directory), "--port", str(port)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        try:
            wait_ready(process, port)
            verify_server_identity(port)
            schema_url = f"http://127.0.0.1:{port}/schema?table=events"
            with NO_REDIRECT_OPENER.open(schema_url, timeout=10) as response:
                schema = json.load(response)
            if schema.get("rows") != 3:
                raise ValueError(f"unexpected HTTP schema row count: {schema!r}")
            if [column["name"] for column in schema["columns"]] != ["id", "name"]:
                raise ValueError(f"unexpected HTTP column contract: {schema!r}")

            for query in QUERIES:
                local = cli(binary, directory, query)
                remote = request(port, query)
                if local != remote:
                    raise ValueError(
                        f"CLI/HTTP mismatch for {query}: CLI={local!r}, HTTP={remote!r}"
                    )

            try:
                request(port, "INSERT INTO events VALUES (4, 'Denied')")
            except urllib.error.HTTPError as error:
                if error.code < 400:
                    raise AssertionError(f"unexpected HTTP status: {error.code}")
            else:
                raise AssertionError("HTTP accepted a write operation")
            if cli(binary, directory, "SELECT count(*) FROM events") != b"count\n3\n":
                raise AssertionError("rejected HTTP write changed the database")
            return {"queries_checked": len(QUERIES), "rows": 3, "http_read_only": True}
        finally:
            process.terminate()
            try:
                process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate(timeout=5)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--server", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        report = check_parity(args.binary, args.server)
    except (OSError, ValueError, RuntimeError, AssertionError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"CLI/HTTP parity failed: {error}\n")
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())

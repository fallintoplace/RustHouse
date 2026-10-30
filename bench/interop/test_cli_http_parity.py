"""Contract tests for the disposable RustHouse CLI/HTTP parity server."""

import json
import unittest
from unittest import mock

from bench.interop import cli_http_parity as parity


class InteropParityTests(unittest.TestCase):
    def test_redirect_requests_are_not_forwarded(self):
        handler = parity.NoRedirect()
        self.assertIsNone(handler.redirect_request(
            None, None, 302, "Found", {}, "http://example.org/stolen"
        ))

    def test_server_identity_rejects_wrong_or_malformed_capabilities(self):
        invalid = (
            [],
            {"name": "Unknown", "source": "database", "read_only": True},
            {"name": "RustHouse", "source": "generated", "read_only": True},
            {"name": "RustHouse", "source": "database", "read_only": False},
        )
        for payload in invalid:
            with self.subTest(payload=payload), mock.patch.object(parity, "NO_REDIRECT_OPENER") as opener:
                opener.open.return_value.__enter__.return_value.read.return_value = json.dumps(payload).encode()
                with self.assertRaisesRegex(ValueError, "capabilities"):
                    parity.verify_server_identity(8123)

    def test_server_identity_accepts_bounded_database_capabilities(self):
        payload = {"name": "RustHouse", "source": "database", "read_only": True}
        with mock.patch.object(parity, "NO_REDIRECT_OPENER") as opener:
            response = opener.open.return_value.__enter__.return_value
            response.read.return_value = json.dumps(payload).encode()
            self.assertEqual(parity.verify_server_identity(8123), payload)
            response.read.assert_called_once_with(1_048_577)

    def test_http_response_is_bounded(self):
        with mock.patch.object(parity, "NO_REDIRECT_OPENER") as opener:
            response = opener.open.return_value.__enter__.return_value
            response.status = 200
            response.read.return_value = b"x" * (4 * 1024 * 1024 + 1)
            with self.assertRaisesRegex(ValueError, "exceeds 4 MiB"):
                parity.request(8123, "SELECT 1")
            response.read.assert_called_once_with(4 * 1024 * 1024 + 1)

    def test_post_query_targets_the_verified_opener(self):
        with mock.patch.object(parity, "NO_REDIRECT_OPENER") as opener:
            response = opener.open.return_value.__enter__.return_value
            response.status = 200
            response.read.return_value = b"one\\n"
            self.assertEqual(parity.request(8123, "SELECT 1"), b"one\\n")
            req = opener.open.call_args.args[0]
            self.assertEqual(req.get_method(), "POST")
            self.assertEqual(req.data, b"SELECT 1")




class ServerStderrBackpressureTests(unittest.TestCase):
    def test_server_launch_does_not_create_unread_pipe(self):
        with mock.patch.object(parity.subprocess, "Popen") as popen:
            parity.launch_server("server", "db", 8123)
            self.assertEqual(popen.call_args.kwargs["stderr"], parity.subprocess.DEVNULL)
            self.assertEqual(popen.call_args.kwargs["stdout"], parity.subprocess.DEVNULL)
            self.assertEqual(popen.call_args.args[0],
                             ["server", "--database", "db", "--port", "8123"])



class BoundedCliParityTests(unittest.TestCase):
    def test_large_cli_output_is_rejected_without_pipe_capture(self):
        from types import SimpleNamespace

        def run(_command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            self.assertTrue(hasattr(kwargs["stderr"], "write"))
            kwargs["stdout"].truncate(4 * 1024 * 1024 + 1)
            return SimpleNamespace(returncode=0)

        with mock.patch.object(parity.subprocess, "run", side_effect=run):
            with self.assertRaisesRegex(ValueError, "4 MiB"):
                parity.cli("binary", "db", "SELECT 1")

    def test_error_message_is_bounded_and_no_stderr_pipe(self):
        from types import SimpleNamespace

        def run(_command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            kwargs["stderr"].write(b"x" * 10000 + b"last failure")
            return SimpleNamespace(returncode=1)

        with mock.patch.object(parity.subprocess, "run", side_effect=run):
            with self.assertRaisesRegex(RuntimeError, "last failure") as raised:
                parity.cli("binary", "db", "SELECT 1")
        self.assertLess(len(str(raised.exception)), 550)

    def test_normal_cli_output_is_unchanged(self):
        from types import SimpleNamespace

        def run(_command, **kwargs):
            kwargs["stdout"].write(b"count\\n3\\n")
            return SimpleNamespace(returncode=0)

        with mock.patch.object(parity.subprocess, "run", side_effect=run):
            self.assertEqual(parity.cli("binary", "db", "SELECT count(*) FROM events"),
                             b"count\\n3\\n")


class SchemaResponseLimitTests(unittest.TestCase):
    def test_oversized_schema_is_rejected_before_json_decode(self):
        with mock.patch.object(parity, "NO_REDIRECT_OPENER") as opener:
            response = opener.open.return_value.__enter__.return_value
            response.read.return_value = b" " * 1_048_577
            with self.assertRaisesRegex(ValueError, "exceeds 1 MiB"):
                parity.fetch_schema(8123)
            response.read.assert_called_once_with(1_048_577)

    def test_schema_requires_object_and_accepts_normal_json(self):
        with mock.patch.object(parity, "NO_REDIRECT_OPENER") as opener:
            response = opener.open.return_value.__enter__.return_value
            response.read.return_value = b'[]'
            with self.assertRaisesRegex(ValueError, "JSON object"):
                parity.fetch_schema(8123)
            response.read.return_value = b'{"rows":3,"columns":[]}'
            self.assertEqual(parity.fetch_schema(8123)["rows"], 3)

if __name__ == "__main__":
    unittest.main()

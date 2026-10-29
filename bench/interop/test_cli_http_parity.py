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

    def test_post_query_targets_the_verified_opener(self):
        with mock.patch.object(parity, "NO_REDIRECT_OPENER") as opener:
            response = opener.open.return_value.__enter__.return_value
            response.status = 200
            response.read.return_value = b"one\\n"
            self.assertEqual(parity.request(8123, "SELECT 1"), b"one\\n")
            req = opener.open.call_args.args[0]
            self.assertEqual(req.get_method(), "POST")
            self.assertEqual(req.data, b"SELECT 1")


if __name__ == "__main__":
    unittest.main()

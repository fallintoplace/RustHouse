import unittest
from unittest.mock import patch
from source_probe import probe, verify_origin

class SourceProbeTests(unittest.TestCase):
    def test_refuses_external_hosts(self):
        with self.assertRaises(ValueError):
            verify_origin("http://example.com:8123")

    @patch("source_probe.read_json")
    def test_direct_probe_refuses_nonloopback_before_http(self, read):
        with self.assertRaisesRegex(ValueError, "loopback"):
            probe("http://example.com:8123", "database")
        read.assert_not_called()

    @patch("source_probe.read_json")
    def test_detects_source_mismatch(self, read):
        read.return_value = {"source": "generated", "read_only": True}
        with self.assertRaises(ValueError):
            probe("http://127.0.0.1:8123", "database")

    @patch("source_probe.scan_rows")
    @patch("source_probe.read_json")
    def test_scan_readiness_requires_query_to_succeed(self, read, scan):
        read.side_effect = [
            {"source": "database", "read_only": True, "version": "0.6"},
            {"columns": [{"name": "id", "type": "UInt64"}]},
        ]
        probe("http://127.0.0.1:8123", "database", "events",
              verify_scan=True, timeout=3)
        scan.assert_called_once_with("http://127.0.0.1:8123", "events", 3)

    @patch("source_probe.read_json")
    def test_scan_requires_a_table(self, read):
        read.return_value = {"source": "database", "read_only": True}
        with self.assertRaisesRegex(ValueError, "requires --table"):
            probe("http://127.0.0.1:8123", "database", verify_scan=True)

    @patch("source_probe.NO_REDIRECT_OPENER")
    def test_scan_rejects_wrong_or_oversized_count_output(self, opener):
        from source_probe import scan_rows
        for body in (b"bad", b"rows_checked\\n-1\\n", b"x" * 129):
            opener.open.return_value.__enter__.return_value.read.return_value = body
            with self.subTest(body=body), self.assertRaisesRegex(ValueError, "scan"):
                scan_rows("http://127.0.0.1:8123", "events", 3)
        opener.open.return_value.__enter__.return_value.read.return_value = (
            b"rows_checked\\n42\\n"
        )
        self.assertEqual(scan_rows("http://127.0.0.1:8123", "events", 3), 42)

    @patch("source_probe.read_json")
    def test_expected_server_version_guards_deployments(self, read):
        read.return_value = {"source": "generated", "read_only": True,
                             "version": "0.6.0"}
        result = probe("http://127.0.0.1:8123", "generated",
                       expected_version="0.6.0")
        self.assertEqual(result["version"], "0.6.0")
        with self.assertRaisesRegex(ValueError, "expected RustHouse version"):
            probe("http://127.0.0.1:8123", "generated", expected_version="0.5.0")
        with self.assertRaisesRegex(ValueError, "expected_version"):
            probe("http://127.0.0.1:8123", "generated", expected_version="")

    @patch("source_probe.read_json")
    def test_rejects_nonobject_capabilities(self, read):
        for payload in (None, [], 123, "unavailable"):
            with self.subTest(payload=payload):
                read.return_value = payload
                with self.assertRaisesRegex(ValueError, "capabilities"):
                    probe("http://127.0.0.1:8123", "database")

    @patch("source_probe.read_json")
    def test_database_schema_required(self, read):
        read.side_effect = [
            {"source": "database", "read_only": True},
            {"columns": [{"name": "id", "type": "UInt64"}]},
        ]
        self.assertEqual(probe("http://127.0.0.1:8123", "database", "events")["source"], "database")


    @patch("source_probe.read_json")
    def test_rejects_invalid_schema_descriptors(self, read):
        for columns in ([], [{"name": "id"}], [{"name": "", "type": "UInt64"}],
                        [{"name": "id", "type": " " }], [
                            {"name": "id", "type": "UInt64"},
                            {"name": "id", "type": "UInt64"}]):
            with self.subTest(columns=columns):
                read.side_effect = [
                    {"source": "database", "read_only": True},
                    {"columns": columns},
                ]
                with self.assertRaises(ValueError):
                    probe("http://127.0.0.1:8123", "database", "items")
    @patch("source_probe.read_json")
    def test_rejects_nonobject_schema_response(self, read):
        read.side_effect = [{"source": "database", "read_only": True}, []]
        with self.assertRaises(ValueError):
            probe("http://127.0.0.1:8123", "database", "items")


    def test_redirect_handler_never_follows_locations(self):
        from source_probe import NoRedirect
        handler = NoRedirect()
        self.assertIsNone(
            handler.redirect_request(None, None, 302, "Found", {}, "http://example.org/")
        )

    @patch("source_probe.NO_REDIRECT_OPENER")
    def test_read_json_uses_restricted_opener(self, opener):
        from source_probe import read_json
        opener.open.return_value.__enter__.return_value.read.return_value = b'{"read_only":true}'
        self.assertEqual(read_json("http://127.0.0.1:8123/capabilities", 3),
                         {"read_only": True})
        opener.open.assert_called_once_with(
            "http://127.0.0.1:8123/capabilities", timeout=3
        )

    @patch("source_probe.NO_REDIRECT_OPENER")
    def test_duplicate_capability_json_keys_are_rejected(self, opener):
        from source_probe import read_json
        opener.open.return_value.__enter__.return_value.read.return_value = (
            b'{"read_only":false,"read_only":true}'
        )
        with self.assertRaisesRegex(ValueError, "duplicate source probe JSON field"):
            read_json("http://127.0.0.1:8123/capabilities", 3)

class SourceTimeoutTests(unittest.TestCase):
    def test_source_probe_rejects_invalid_request_timeouts(self):
        from source_probe import read_json
        with patch("source_probe.NO_REDIRECT_OPENER") as opener:
            for value in (True, 0, -1, float("nan"), float("inf"), 301):
                with self.subTest(timeout=value), self.assertRaisesRegex(ValueError, "timeout"):
                    read_json("http://127.0.0.1:8123/capabilities", value)
            opener.open.assert_not_called()


if __name__ == "__main__":
    unittest.main()

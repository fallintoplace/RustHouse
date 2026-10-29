import unittest
from unittest.mock import patch
from source_probe import probe, verify_origin

class SourceProbeTests(unittest.TestCase):
    def test_refuses_external_hosts(self):
        with self.assertRaises(ValueError):
            verify_origin("http://example.com:8123")

    @patch("source_probe.read_json")
    def test_detects_source_mismatch(self, read):
        read.return_value = {"source": "generated", "read_only": True}
        with self.assertRaises(ValueError):
            probe("http://127.0.0.1:8123", "database")

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

if __name__ == "__main__":
    unittest.main()

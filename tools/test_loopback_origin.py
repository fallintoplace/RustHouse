"""All diagnostic clients reject ambiguous or invalid loopback HTTP origins."""
import unittest
from unittest.mock import patch

import cluster_health
import schema_diff
import source_probe
from loopback_origin import require_loopback_http_origin


class CanonicalLoopbackOriginTests(unittest.TestCase):
    def test_accepts_canonical_loopback_origins(self):
        for origin in (
            "http://localhost",
            "http://LOCALHOST:8123/",
            "http://127.0.0.1:1",
            "http://127.0.0.1:65535",
        ):
            with self.subTest(origin=origin):
                self.assertEqual(require_loopback_http_origin(origin), origin.rstrip("/"))

    def test_rejects_ambiguous_authority_and_invalid_ports_before_any_request(self):
        bad = (
            "http://localhost:",
            "http://localhost:0",
            "http://localhost:00080",
            "http://localhost:65536",
            "http://localhost:abc",
            "http://localhost:80@evil.example",
            "http://127.0.0.1:8123/path",
            "http://localhost/?q=1",
            "http://local\nhost:8123",
            "http://127.0.0.1\t:8123",
        )
        with patch("cluster_health.NO_REDIRECT_OPENER") as health, patch(
            "schema_diff.NO_REDIRECT_OPENER"
        ) as schema:
            for origin in bad:
                with self.subTest(origin=origin), self.assertRaisesRegex(ValueError, "loopback"):
                    cluster_health.inspect(origin)
                with self.subTest(origin=origin), self.assertRaisesRegex(ValueError, "loopback"):
                    schema_diff.validate_url(origin)
                with self.subTest(origin=origin), self.assertRaisesRegex(ValueError, "loopback"):
                    source_probe.verify_origin(origin)
            health.open.assert_not_called()
            schema.open.assert_not_called()


if __name__ == "__main__":
    unittest.main()

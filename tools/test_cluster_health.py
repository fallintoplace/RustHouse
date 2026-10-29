import unittest
from cluster_health import validate

class ClusterTests(unittest.TestCase):
    def test_all_healthy(self):
        self.assertTrue(validate({"total": 1, "healthy": 1,
                                  "peers": [{"address": "127.0.0.1:8124", "healthy": True}]}))

    def test_partial_failure(self):
        self.assertFalse(validate({"total": 1, "healthy": 0,
                                   "peers": [{"address": "127.0.0.1:8124", "healthy": False}]}))

    def test_explicit_healthy_peer_threshold(self):
        payload = {"total": 3, "healthy": 2, "peers": [
            {"address": "a", "healthy": True},
            {"address": "b", "healthy": True},
            {"address": "c", "healthy": False},
        ]}
        self.assertFalse(validate(payload))
        self.assertTrue(validate(payload, min_healthy=2))
        self.assertFalse(validate(payload, min_healthy=3))
        with self.assertRaisesRegex(ValueError, "min_healthy"):
            validate(payload, min_healthy=4)
        with self.assertRaisesRegex(ValueError, "min_healthy"):
            validate(payload, min_healthy=True)

    def test_inconsistent_count_rejected(self):
        with self.assertRaises(ValueError):
            validate({"total": 1, "healthy": 1,
                      "peers": [{"address": "127.0.0.1:8124", "healthy": False}]})


    def test_redirect_handler_refuses_destination(self):
        from cluster_health import NoRedirect
        self.assertIsNone(NoRedirect().redirect_request(None, None, 301, "Moved", {}, "http://example.org/"))

    def test_inspect_uses_redirect_rejecting_opener(self):
        from unittest.mock import patch
        from cluster_health import inspect
        payload = b'{"total":0,"healthy":0,"peers":[]}'
        with patch("cluster_health.NO_REDIRECT_OPENER") as opener:
            opener.open.return_value.__enter__.return_value.read.return_value = payload
            healthy, info = inspect("http://127.0.0.1:8123", timeout=3)
            self.assertTrue(healthy)
            self.assertEqual(info["total"], 0)
            opener.open.assert_called_once_with("http://127.0.0.1:8123/cluster/health", timeout=3)

if __name__ == "__main__":
    unittest.main()

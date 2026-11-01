import unittest
from cluster_health import validate

class ClusterTests(unittest.TestCase):
    def test_all_healthy(self):
        self.assertTrue(validate({"total": 1, "healthy": 1,
                                  "peers": [{"address": "127.0.0.1:8124", "healthy": True}]}))

    def test_empty_cluster_is_not_ready(self):
        self.assertFalse(validate({"total": 0, "healthy": 0, "peers": []}))

    def test_exact_membership_required_even_if_all_reported_healthy(self):
        payload = {"total": 2, "healthy": 2, "peers": [
            {"address": "127.0.0.1:9001", "healthy": True},
            {"address": "127.0.0.1:9002", "healthy": True},
        ]}
        self.assertTrue(validate(
            payload, expected_peers=["127.0.0.1:9002", "127.0.0.1:9001"]
        ))
        for expected in (["127.0.0.1:9003", "127.0.0.1:9001"],
                         ["127.0.0.1:9001", "127.0.0.1:9001"], []):
            with self.subTest(expected=expected), self.assertRaisesRegex(
                ValueError, "membership"
            ):
                validate(payload, expected_peers=expected)

    def test_rejects_more_peers_than_the_coordinator_supports(self):
        peers = [{"address": f"127.0.0.1:{9000 + index}", "healthy": True}
                 for index in range(33)]
        with self.assertRaisesRegex(ValueError, "metadata"):
            validate({"total": len(peers), "healthy": len(peers), "peers": peers})

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
            self.assertFalse(healthy)
            self.assertEqual(info["total"], 0)
            opener.open.assert_called_once_with("http://127.0.0.1:8123/cluster/health", timeout=3)

    def test_http_error_response_streams_are_closed(self):
        from io import BytesIO
        from urllib.error import HTTPError
        from unittest.mock import patch
        from cluster_health import inspect
        origin = "http://127.0.0.1:8123"
        payload = BytesIO(b'{"total":0,"healthy":0,"peers":[]}')
        error = HTTPError(origin + "/cluster/health", 503, "Unavailable", {}, payload)
        with patch("cluster_health.NO_REDIRECT_OPENER") as opener:
            opener.open.side_effect = error
            ok, _info = inspect(origin)
        self.assertFalse(ok)
        self.assertTrue(payload.closed)

        payload = BytesIO(b"Not Found")
        error = HTTPError(origin + "/cluster/health", 404, "Not Found", {}, payload)
        with patch("cluster_health.NO_REDIRECT_OPENER") as opener:
            opener.open.side_effect = error
            with self.assertRaises(HTTPError):
                inspect(origin)
        self.assertTrue(payload.closed)

    def test_duplicate_peer_addresses_cannot_pass_health_check(self):
        with self.assertRaisesRegex(ValueError, "duplicate peer"):
            validate({"total": 2, "healthy": 2, "peers": [
                {"address": "127.0.0.1:9000", "healthy": True},
                {"address": "127.0.0.1:9000", "healthy": True},
            ]})

    def test_whitespace_peer_addresses_are_invalid(self):
        for value in ("", "  "):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "invalid peer"):
                validate({"total": 1, "healthy": 1,
                          "peers": [{"address": value, "healthy": True}]})

    def test_cluster_health_refuses_invalid_timeouts_without_http(self):
        from unittest.mock import patch
        from cluster_health import inspect
        with patch("cluster_health.NO_REDIRECT_OPENER") as opener:
            for value in (True, 0, -1, float("nan"), float("inf"), 301):
                with self.subTest(timeout=value), self.assertRaisesRegex(ValueError, "timeout"):
                    inspect("http://127.0.0.1:8123", timeout=value)
            opener.open.assert_not_called()

    def test_duplicate_json_keys_fail_closed(self):
        from cluster_health import inspect
        from unittest.mock import patch
        raw = b'{"total":0,"total":1,"healthy":0,"peers":[]}'
        with patch("cluster_health.NO_REDIRECT_OPENER") as opener:
            opener.open.return_value.__enter__.return_value.read.return_value = raw
            with self.assertRaisesRegex(ValueError, "duplicate peer-health JSON field"):
                inspect("http://127.0.0.1:8123")

    def test_http_503_never_passes_even_with_all_peers_healthy(self):
        from io import BytesIO
        from urllib.error import HTTPError
        from unittest.mock import patch
        from cluster_health import inspect

        origin = "http://127.0.0.1:8123"
        raw = BytesIO(b'{"total":1,"healthy":1,"peers":[{"address":"127.0.0.1:8124","healthy":true}]}')
        error = HTTPError(origin + "/cluster/health", 503, "Unavailable", {}, raw)
        with patch("cluster_health.NO_REDIRECT_OPENER") as opener:
            opener.open.side_effect = error
            ok, payload = inspect(origin, min_healthy=1)
        self.assertFalse(ok)
        self.assertEqual(payload["healthy"], 1)
        self.assertTrue(raw.closed)

if __name__ == "__main__":
    unittest.main()

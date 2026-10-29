import unittest
from cluster_health import validate

class ClusterTests(unittest.TestCase):
    def test_all_healthy(self):
        self.assertTrue(validate({"total": 1, "healthy": 1,
                                  "peers": [{"address": "127.0.0.1:8124", "healthy": True}]}))

    def test_partial_failure(self):
        self.assertFalse(validate({"total": 1, "healthy": 0,
                                   "peers": [{"address": "127.0.0.1:8124", "healthy": False}]}))

    def test_inconsistent_count_rejected(self):
        with self.assertRaises(ValueError):
            validate({"total": 1, "healthy": 1,
                      "peers": [{"address": "127.0.0.1:8124", "healthy": False}]})


    def test_duplicate_peers_cannot_pass_health_check(self):
        with self.assertRaisesRegex(ValueError, "duplicate peer"):
            validate({"total": 2, "healthy": 2, "peers": [
                {"address": "127.0.0.1:9000", "healthy": True},
                {"address": "127.0.0.1:9000", "healthy": True},
            ]})

    def test_empty_peer_address_cannot_count_as_healthy(self):
        for value in ("", "  "):
            with self.assertRaisesRegex(ValueError, "invalid peer"):
                validate({"total": 1, "healthy": 1,
                          "peers": [{"address": value, "healthy": True}]})


if __name__ == "__main__":
    unittest.main()

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


if __name__ == "__main__":
    unittest.main()

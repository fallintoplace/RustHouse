import unittest

from tools.cluster_health import inspect


class ClusterHealthOriginTests(unittest.TestCase):
    def test_rejects_loopback_origins_with_credentials(self):
        for value in ("http://user@127.0.0.1:8123",
                      "http://user:secret@localhost:8123",
                      "http://:password@127.0.0.1:8123"):
            with self.subTest(url=value), self.assertRaisesRegex(ValueError, "loopback"):
                inspect(value)

    def test_rejects_external_host(self):
        with self.assertRaisesRegex(ValueError, "loopback"):
            inspect("http://example.org:8123")


if __name__ == "__main__":
    unittest.main()

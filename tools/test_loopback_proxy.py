"""Loopback-only RustHouse tools must never delegate requests to proxies."""
import unittest
from urllib.request import ProxyHandler

import cluster_health
import schema_diff
import source_probe


class LoopbackProxyTests(unittest.TestCase):
    def test_all_probe_clients_bypass_environment_proxy_settings(self):
        for tool in (cluster_health, schema_diff, source_probe):
            with self.subTest(tool=tool.__name__):
                proxies = [
                    handler for handler in tool.NO_REDIRECT_OPENER.handlers
                    if isinstance(handler, ProxyHandler)
                ]
                self.assertEqual(len(proxies), 1)
                self.assertEqual(proxies[0].proxies, {})


if __name__ == "__main__":
    unittest.main()

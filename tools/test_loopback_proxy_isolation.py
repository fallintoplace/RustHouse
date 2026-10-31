import unittest
from urllib.request import ProxyHandler

import cluster_health
import schema_diff
import source_probe


class LoopbackProxyIsolationTests(unittest.TestCase):
    def test_every_probe_bypasses_environment_proxies(self):
        for module in (cluster_health, schema_diff, source_probe):
            with self.subTest(module=module.__name__):
                handlers = [
                    handler for handler in module.NO_REDIRECT_OPENER.handlers
                    if isinstance(handler, ProxyHandler)
                ]
                self.assertEqual(len(handlers), 1)
                self.assertEqual(handlers[0].proxies, {})


if __name__ == "__main__":
    unittest.main()

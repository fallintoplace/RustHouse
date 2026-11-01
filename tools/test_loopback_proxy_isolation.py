import unittest
from unittest.mock import patch
from urllib.request import ProxyHandler, build_opener

import cluster_health
import schema_diff
import source_probe


class LoopbackProxyIsolationTests(unittest.TestCase):
    def test_every_probe_bypasses_environment_proxies(self):
        with patch("urllib.request.getproxies", return_value={
            "http": "http://proxy.example:3128"
        }):
            default = build_opener()
            self.assertTrue(any(
                isinstance(handler, ProxyHandler) for handler in default.handlers
            ))
            for module in (cluster_health, schema_diff, source_probe):
                with self.subTest(module=module.__name__):
                    # An empty ProxyHandler suppresses the default proxy
                    # handler. urllib discards it because there are no proxies.
                    opener = build_opener(ProxyHandler({}), module.NoRedirect())
                    self.assertFalse(any(
                        isinstance(handler, ProxyHandler) for handler in opener.handlers
                    ))
                    self.assertFalse(any(
                        isinstance(handler, ProxyHandler)
                        for handler in module.NO_REDIRECT_OPENER.handlers
                    ))


if __name__ == "__main__":
    unittest.main()

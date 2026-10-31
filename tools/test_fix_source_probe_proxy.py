import unittest
from urllib.request import ProxyHandler

from source_probe import NO_REDIRECT_OPENER


class LoopbackProxyIsolationTests(unittest.TestCase):
    def test_health_probe_ignores_environment_proxy_configuration(self):
        handlers = [handler for handler in NO_REDIRECT_OPENER.handlers
                    if isinstance(handler, ProxyHandler)]
        self.assertEqual(len(handlers), 1)
        self.assertEqual(handlers[0].proxies, {})


if __name__ == "__main__":
    unittest.main()

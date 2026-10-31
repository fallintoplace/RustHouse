import unittest
from urllib.request import ProxyHandler

from source_probe import NO_REDIRECT_OPENER


class LoopbackProxyIsolationTests(unittest.TestCase):
    def test_health_probe_ignores_environment_proxy_configuration(self):
        handlers = [handler for handler in NO_REDIRECT_OPENER.handlers
                    if isinstance(handler, ProxyHandler)]
        # urllib deliberately omits an empty ProxyHandler from the opener.
        # Its absence is what prevents fallback to proxy environment values.
        self.assertEqual(handlers, [])


if __name__ == "__main__":
    unittest.main()

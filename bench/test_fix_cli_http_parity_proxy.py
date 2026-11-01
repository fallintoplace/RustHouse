import unittest
from urllib.request import ProxyHandler

from bench.interop import cli_http_parity


class ParityProxyIsolationTests(unittest.TestCase):
    def test_loopback_http_requests_ignore_environment_proxies(self):
        handlers = [handler for handler in cli_http_parity.NO_REDIRECT_OPENER.handlers
                    if isinstance(handler, ProxyHandler)]
        self.assertEqual(len(handlers), 1)
        self.assertEqual(handlers[0].proxies, {})


if __name__ == "__main__":
    unittest.main()

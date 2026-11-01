import importlib
import unittest
from unittest.mock import patch
from urllib.request import ProxyHandler

from bench.interop import cli_http_parity


class ParityProxyIsolationTests(unittest.TestCase):
    def test_loopback_http_requests_ignore_environment_proxies(self):
        # ProxyHandler({}) suppresses the default proxy handler, but is itself
        # left out of OpenerDirector.handlers because it handles no protocols.
        try:
            with patch("urllib.request.getproxies",
                       return_value={"http": "http://proxy.invalid:8080"}):
                importlib.reload(cli_http_parity)
                handlers = [handler for handler in cli_http_parity.NO_REDIRECT_OPENER.handlers
                            if isinstance(handler, ProxyHandler)]
                self.assertEqual(handlers, [])
        finally:
            importlib.reload(cli_http_parity)


if __name__ == "__main__":
    unittest.main()

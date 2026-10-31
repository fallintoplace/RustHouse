"""Loopback-only RustHouse tools must never delegate requests to proxies."""
import os
import runpy
import unittest
from unittest.mock import patch
from urllib.request import ProxyHandler

import cluster_health
import schema_diff
import source_probe


class LoopbackProxyTests(unittest.TestCase):
    def test_all_probe_clients_bypass_environment_proxy_settings(self):
        # Load each opener with explicit proxies in the environment. The
        # default urllib opener would install a proxy handler in this case.
        proxy_env = {
            "http_proxy": "http://127.0.0.1:1",
            "https_proxy": "http://127.0.0.1:1",
            "HTTP_PROXY": "http://127.0.0.1:1",
            "HTTPS_PROXY": "http://127.0.0.1:1",
            "no_proxy": "",
            "NO_PROXY": "",
        }
        with patch.dict(os.environ, proxy_env):
            for tool in (cluster_health, schema_diff, source_probe):
                with self.subTest(tool=tool.__name__):
                    opener = runpy.run_path(tool.__file__)["NO_REDIRECT_OPENER"]
                    self.assertFalse(any(
                        isinstance(handler, ProxyHandler) for handler in opener.handlers
                    ), f"{tool.__name__} inherited an HTTP proxy")


if __name__ == "__main__":
    unittest.main()

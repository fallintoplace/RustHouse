import unittest
from unittest.mock import patch
from smoke import NoRedirect, check, origin


class SmokeTests(unittest.TestCase):
    def test_refuses_http_redirect_targets(self):
        self.assertIsNone(
            NoRedirect().redirect_request(None, None, 307, "redirect", {},
                                          "http://example.net/")
        )


    def test_refuses_public_endpoint(self):
        with self.assertRaises(ValueError):
            origin("http://public.example:8123")
        self.assertEqual(origin("http://10.0.0.1:8123", True), "http://10.0.0.1:8123")

    @patch("smoke.request")
    def test_reference_success(self, request):
        request.side_effect = [
            "Ok.\n", '{"version":"0.1","read_only":true}',
            '[{"number":"0"},{"number":"1"},{"number":"2"}]',
            "Logical plan (no data read)",
        ]
        self.assertEqual(check("http://127.0.0.1:8123")["version"], "0.1")

    @patch("smoke.request")
    def test_incorrect_query_is_failure(self, request):
        request.side_effect = ["Ok.\n", '{"read_only":true}', '[{"number":0}]']
        with self.assertRaises(ValueError):
            check("http://127.0.0.1:8123")


    def test_private_mode_never_accepts_public_hosts_or_dns(self):
        for hostname in ("example.org", "8.8.4.4", "169.254.1.1",
                         "127.0.0.2", "0.0.0.0", "[::1]"):
            with self.subTest(host=hostname), self.assertRaises(ValueError):
                origin("http://" + hostname + ":8123", True)
        for hostname in ("10.1.2.3", "172.16.0.1", "172.31.255.254",
                         "192.168.1.1", "100.64.0.1"):
            self.assertEqual(origin("http://" + hostname + ":8123", True),
                             "http://" + hostname + ":8123")

    def test_smoke_client_ignores_environment_proxies(self):
        import smoke
        from urllib.request import ProxyHandler
        proxies = [handler for handler in smoke.urlopen.__self__.handlers
                   if isinstance(handler, ProxyHandler)]
        self.assertEqual(len(proxies), 1)
        self.assertEqual(proxies[0].proxies, {})


if __name__ == "__main__":
    unittest.main()

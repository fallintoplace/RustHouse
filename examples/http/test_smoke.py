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


if __name__ == "__main__":
    unittest.main()

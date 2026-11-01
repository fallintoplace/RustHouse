import io
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

from rusthouse_client import NoRedirect, RustHouseClient, RustHouseError, RustHouseHTTPError


class Reply:
    def __init__(self, body):
        self.body = io.BytesIO(body)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        pass

    def read(self, size=-1):
        return self.body.read(size)


class ClientTests(unittest.TestCase):
    def test_redirect_handler_never_forwards_requests(self):
        handler = NoRedirect()
        for target in ("http://example.org/", "http://127.0.0.1:9000/"):
            self.assertIsNone(handler.redirect_request(None, None, 307, "redirect", {}, target))


    def test_avoids_exposing_unauthenticated_server(self):
        with self.assertRaises(ValueError):
            RustHouseClient("http://database.example:8123")
        with self.assertRaises(ValueError):
            RustHouseClient("https://127.0.0.1:8123")
        with self.assertRaises(ValueError):
            RustHouseClient("http://127.0.0.1:8123/?token=secret")
        RustHouseClient("http://10.0.0.1:8123", allow_private_network=True)

    @patch("rusthouse_client.urlopen")
    def test_query_is_posted_without_interpolating_url(self, open_url):
        open_url.return_value = Reply(b'[{"number":"18446744073709551615"}]')
        client = RustHouseClient()
        rows = client.query("SELECT number FROM numbers(1)")
        self.assertEqual(rows[0]["number"], "18446744073709551615")
        request = open_url.call_args.args[0]
        self.assertEqual(request.get_method(), "POST")
        self.assertEqual(request.full_url, "http://127.0.0.1:8123/?format=json")
        self.assertEqual(request.data, b"SELECT number FROM numbers(1)")
        self.assertEqual(open_url.call_args.kwargs["timeout"], 5)

    @patch("rusthouse_client.urlopen")
    def test_schema_encodes_identifier(self, open_url):
        open_url.return_value = Reply(b'{"columns":[]}')
        RustHouseClient().schema("team&other")
        self.assertTrue(open_url.call_args.args[0].full_url.endswith("?table=team%26other"))

    @patch("rusthouse_client.urlopen")
    def test_http_error_has_status(self, open_url):
        open_url.side_effect = HTTPError(
            "http://127.0.0.1:8123/", 409, "Conflict", {}, io.BytesIO(b"busy")
        )
        with self.assertRaises(RustHouseHTTPError) as ctx:
            RustHouseClient().query("SELECT 1")
        self.assertEqual(ctx.exception.status, 409)

    @patch("rusthouse_client.urlopen")
    def test_invalid_json_is_rejected(self, open_url):
        open_url.return_value = Reply(b"{broken")
        with self.assertRaises(RustHouseError):
            RustHouseClient().capabilities()

    def test_sql_is_bounded_in_bytes(self):
        with self.assertRaises(ValueError):
            RustHouseClient().query("SELECT '" + "é" * 3000 + "'")


    def test_no_environment_proxy_handler_is_installed(self):
        from urllib.request import ProxyHandler
        import rusthouse_client

        handlers = rusthouse_client.urlopen.__self__.handlers
        proxy_handlers = [handler for handler in handlers
                          if isinstance(handler, ProxyHandler)]
        self.assertEqual(len(proxy_handlers), 1)
        self.assertEqual(proxy_handlers[0].proxies, {})

    @patch("rusthouse_client.urlopen")
    def test_explain_is_bounded_like_json_queries(self, open_url):
        open_url.return_value = Reply(b"x" * (16 * 1024 * 1024 + 1))
        with self.assertRaisesRegex(RustHouseError, "16 MiB"):
            RustHouseClient().explain("SELECT 1")


if __name__ == "__main__":
    unittest.main()

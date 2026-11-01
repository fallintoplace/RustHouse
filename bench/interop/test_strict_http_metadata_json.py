import unittest

from bench.interop.cli_http_parity import _strict_response_json


class ParityStrictJsonTests(unittest.TestCase):
    def test_rejects_nonstandard_metadata_in_http_responses(self):
        for token in ("NaN", "Infinity", "-Infinity"):
            with self.subTest(token=token), self.assertRaisesRegex(ValueError, "nonstandard"):
                _strict_response_json(('{"source":"database","ignored":' + token + '}').encode())

    def test_rejects_duplicate_fields_and_keeps_normal_payloads(self):
        with self.assertRaisesRegex(ValueError, "duplicate"):
            _strict_response_json(b'{"source":"database","source":"generated"}')
        self.assertEqual(_strict_response_json(b'{"read_only":true}'), {"read_only": True})


if __name__ == "__main__":
    unittest.main()

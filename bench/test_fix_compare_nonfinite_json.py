"""Reject nonstandard JSON numbers in benchmark comparison inputs."""
import tempfile
import unittest
from pathlib import Path

from bench.compare import load_report


class CompareInputNumberTests(unittest.TestCase):
    def test_nonfinite_tokens_never_load(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "input.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"results": [], "bad": ' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        load_report(path)

    def test_finite_numbers_keep_their_values(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "input.json"
            path.write_text('{"median": 0.125, "results": []}', encoding="utf-8")
            self.assertEqual(load_report(path)["median"], 0.125)


if __name__ == "__main__":
    unittest.main()

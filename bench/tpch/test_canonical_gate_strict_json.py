import tempfile
import unittest
from pathlib import Path
from bench.tpch.canonical_gate import parse_report


class StrictTpchJsonTests(unittest.TestCase):
    def test_rejects_nonstandard_constants_and_duplicate_keys(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"reference_checked":true,"metadata":' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        parse_report(path)
            path.write_text('{"query_sha256":"a","query_sha256":"b"}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate"):
                parse_report(path)


if __name__ == "__main__":
    unittest.main()

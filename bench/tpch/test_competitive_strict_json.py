import tempfile
import unittest
from pathlib import Path
from bench.tpch.competitive_gate import load_report


class StrictTpchJsonTests(unittest.TestCase):
    def test_rejects_nonstandard_constants_and_duplicate_keys(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"workload":"paired","metadata":' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        load_report(path)
            path.write_text('{"query_sha256":"a","query_sha256":"b"}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate"):
                load_report(path)


if __name__ == "__main__":
    unittest.main()

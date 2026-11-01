import tempfile
import unittest
from pathlib import Path
from bench.scale.physical_gate import parse_report


class StrictJsonReportTests(unittest.TestCase):
    def test_rejects_nonstandard_numeric_tokens(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"workload":"scan","unused":' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        parse_report(path)

    def test_standard_json_remains_valid(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            path.write_text('{"ok":true}', encoding="utf-8")
            self.assertEqual(parse_report(path), {"ok": True})


if __name__ == "__main__":
    unittest.main()

import tempfile
import unittest
from pathlib import Path
from bench.validate_benchmark import load_manifest


class StrictJsonReportTests(unittest.TestCase):
    def test_rejects_nonstandard_numeric_tokens(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"schema_version":1,"unused":' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        load_manifest(path)

    def test_standard_json_remains_valid(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            path.write_text('{"ok":true}', encoding="utf-8")
            self.assertEqual(load_manifest(path), {"ok": True})


if __name__ == "__main__":
    unittest.main()

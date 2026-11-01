import tempfile
import unittest
from pathlib import Path
from bench.scale.compare_physical import parse_report


class PhysicalCompareStrictJsonTests(unittest.TestCase):
    def test_rejects_nonstandard_unused_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "comparison.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"workload":"physical","unused":' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        parse_report(path)

    def test_preserves_standard_reports(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "comparison.json"
            path.write_text('{"rows_written_and_verified":3}', encoding="utf-8")
            self.assertEqual(parse_report(path), {"rows_written_and_verified": 3})


if __name__ == "__main__":
    unittest.main()

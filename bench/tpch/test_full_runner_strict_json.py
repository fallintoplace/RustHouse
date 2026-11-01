import tempfile
import unittest
from pathlib import Path
from run_canonical import parse_report


class FullTpchRunnerJsonTests(unittest.TestCase):
    def test_rejects_nonstandard_constants_and_duplicate_results(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "full.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"results":[],"metadata":' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        parse_report(path)
            path.write_text('{"results":[],"results":[1]}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate"):
                parse_report(path)

    def test_valid_json_is_unmodified(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "full.json"
            path.write_text('{"reference_checked":true,"results":[]}', encoding="utf-8")
            self.assertEqual(parse_report(path)["results"], [])


if __name__ == "__main__":
    unittest.main()

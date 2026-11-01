import tempfile
import unittest
from pathlib import Path
from bench.scale.billion_matrix import load_cases


class StrictCaseJsonTests(unittest.TestCase):
    def test_rejects_nonstandard_numbers_before_using_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "01-sample.json"
            for token in ("NaN", "Infinity", "-Infinity"):
                with self.subTest(token=token):
                    path.write_text('{"name":"case","unexpected":' + token + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard"):
                        load_cases(Path(directory))


if __name__ == "__main__":
    unittest.main()

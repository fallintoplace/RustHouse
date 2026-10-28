import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_profile import profile


class TsvProfileDimensionsTests(unittest.TestCase):
    def test_rejects_fractional_and_boolean_counts(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "input.tsv"
            path.write_bytes(b"id\n1\n")
            for kwargs in ({"columns": True}, {"columns": 1.5},
                           {"min_rows": True}, {"min_rows": 0.5},
                           {"header": 1}, {"header": "yes"}):
                with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                    profile(path, **kwargs)

    def test_preserves_header_aware_row_counts(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "input.tsv"
            path.write_bytes(b"id\n1\n")
            self.assertEqual(profile(path, header=True)["rows"], 1)


if __name__ == "__main__":
    unittest.main()

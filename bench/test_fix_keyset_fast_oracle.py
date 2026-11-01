import tempfile
import unittest
from pathlib import Path

from bench.scale.keyset_export import export


class BulkKeysetOracleTests(unittest.TestCase):
    def test_oracle_matches_uint64_max_and_page_boundaries(self):
        for start, count, page_rows in [(0, 1, 1), (17, 8193, 8192),
                                        ((1 << 64) - 50000, 50000, 50000)]:
            with self.subTest(start=start, count=count):
                with tempfile.TemporaryDirectory() as root:
                    destination = Path(root) / "ids.tsv"
                    expected = "".join(f"{number}\n"
                                       for number in range(start, start + count)).encode("ascii")
                    def execute(statement):
                        import re
                        first = int(re.search(r"id >= (\d+)", statement).group(1))
                        chunk = min(page_rows, start + count - first)
                        return "".join(f"{number}\n" for number in
                                       range(first, first + chunk)).encode("ascii")
                    result = export("binary", "db", "items", destination, start, count,
                                    page_rows=page_rows, execute=execute)
                    self.assertEqual(destination.read_bytes(), expected)
                    self.assertEqual(result["rows"], count)

    def test_one_wrong_row_still_rejects_the_export(self):
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / "keys.tsv"
            with self.assertRaisesRegex(ValueError, "missing, duplicated or unordered"):
                export("binary", "db", "items", output, 5, 3,
                       execute=lambda _: b"5\n5\n7\n")
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()

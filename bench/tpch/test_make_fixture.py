import tempfile
import unittest
from pathlib import Path

from make_fixture import generate, row_source

class FixtureTests(unittest.TestCase):
    def test_seeded_fixture_is_repeatable_and_has_a_known_match(self):
        self.assertEqual(list(row_source(1, 42)), [("1994-06-15", 100000, 600, 10)])
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            one = generate(Path(first), 120, 9)
            two = generate(Path(second), 120, 9)
            self.assertEqual(one, two)
            self.assertGreaterEqual(one["matched_rows"], 1)
            self.assertEqual(
                (Path(first) / "lineitem.tsv").read_bytes(),
                (Path(second) / "lineitem.tsv").read_bytes(),
            )
            rows = (Path(first) / "lineitem.tsv").read_text().splitlines()
            self.assertEqual(len(rows), 121)
            expected = f"{one['revenue_bps']}\t{one['matched_rows']}\n"
            self.assertEqual(
                (Path(first) / "references" / "q06.tsv").read_text(), expected
            )

    def test_rejects_empty_input(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(ValueError):
                generate(Path(root), 0, 0)

if __name__ == "__main__":
    unittest.main()

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

    def test_large_fixture_preserves_exact_rows_and_revenue_oracle(self):
        with tempfile.TemporaryDirectory() as root:
            report = generate(Path(root), 4097, 21)
            expected = ["l_shipdate\tl_extendedprice\tl_discount\tl_quantity"]
            count = revenue = 0
            for shipdate, price, discount, quantity in row_source(4097, 21):
                expected.append(f"{shipdate}\t{price}\t{discount}\t{quantity}")
                if ("1994-01-01" <= shipdate < "1995-01-01"
                        and 500 <= discount <= 700 and quantity < 24):
                    count += 1
                    revenue += price * discount
            self.assertEqual((Path(root) / "lineitem.tsv").read_text().splitlines(),
                             expected)
            self.assertEqual(report["matched_rows"], count)
            self.assertEqual(report["revenue_bps"], revenue)

    def test_rejects_empty_input(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(ValueError):
                generate(Path(root), 0, 0)

if __name__ == "__main__":
    unittest.main()

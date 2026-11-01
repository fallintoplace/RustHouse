"""Fixture TSV record budgets are measured in encoded bytes."""
import io
import unittest

from load_fixture import bounded_tsv_records


class FixtureRecordBudgetTests(unittest.TestCase):
    def test_rejects_unicode_rows_exceeding_byte_budget(self):
        line = "é" * 32768 + "\n"
        self.assertLess(len(line), 65_536)
        with self.assertRaisesRegex(ValueError, "64 KiB"):
            list(bounded_tsv_records(io.StringIO(line)))

    def test_accepts_exact_budget_and_rejects_truncation(self):
        exact = "x" * 65_535 + "\n"
        self.assertEqual(list(bounded_tsv_records(io.StringIO(exact))), [exact])
        with self.assertRaisesRegex(ValueError, "truncated"):
            list(bounded_tsv_records(io.StringIO("x" * 65_535)))


if __name__ == "__main__":
    unittest.main()

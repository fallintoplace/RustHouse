"""CSV field budgets have clear type and range diagnostics."""
import unittest

from bench.scale.csv_stream_to_tsv import convert


class CsvFieldDiagnosticTests(unittest.TestCase):
    def test_malformed_budget_fails_before_opening_source(self):
        for invalid in (True, 1.5, float("nan"), "1024", 0, 8388609):
            with self.subTest(value=invalid), self.assertRaisesRegex(
                    ValueError, "positive integer"):
                convert("missing.csv", "out.tsv", max_field_bytes=invalid)


if __name__ == "__main__":
    unittest.main()

import unittest
from decimal import Decimal

from numeric_oracle import compare_tsv, numeric_equal, parse_tolerance


class NumericOracleTests(unittest.TestCase):
    def test_numeric_oracle_bounds_file_reads(self):
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        from numeric_oracle import read_input
        with tempfile.TemporaryDirectory() as root:
            file = Path(root) / "result.tsv"
            file.write_bytes(b"0123456789abcdefg")
            with patch("numeric_oracle.MAX_INPUT_BYTES", 16):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    read_input(file)
            self.assertEqual(read_input(file), b"0123456789abcdefg")


    def test_decimal_precision_and_tolerance(self):
        self.assertTrue(numeric_equal(b"1.00", b"1.0"))
        self.assertTrue(numeric_equal(b"1e-9", b"0.000000001"))
        self.assertTrue(numeric_equal(b"999999999999999999999999.00",
                                      b"999999999999999999999999"))
        self.assertFalse(numeric_equal(b"1.1", b"1.2"))
        self.assertTrue(numeric_equal(b"1.1", b"1.2", Decimal("0.1")))

    def test_nonzero_tolerance_does_not_round_large_decimal_differences(self):
        lower = b"12345678901234567890123456789.01"
        higher_tolerance = Decimal("12345678901234567890123456789.02")
        lower_tolerance = Decimal("12345678901234567890123456789.00")
        self.assertTrue(numeric_equal(b"0", lower, higher_tolerance))
        self.assertFalse(numeric_equal(b"0", lower, lower_tolerance))
        self.assertTrue(compare_tsv(b"0\\n", lower + b"\\n", higher_tolerance))
        self.assertFalse(numeric_equal(b"0", b"1e-10000", Decimal("0.01")))

    def test_extreme_decimal_exponents_do_not_match_or_crash(self):
        self.assertFalse(numeric_equal(b"1e-9999999999", b"0"))
        self.assertTrue(numeric_equal(b"1e999999999999", b"1.0e999999999999"))
        self.assertFalse(numeric_equal(b"1e999999999999", b"0", Decimal("1")))
        self.assertTrue(numeric_equal(b"0.0010", b"0.001", Decimal("0")))

    def test_text_and_structure_stay_strict(self):
        self.assertTrue(compare_tsv(b"1.00\tEUROPE\n", b"1\tEUROPE\n"))
        self.assertFalse(compare_tsv(b"1\tEUROPE\n", b"1\tASIA\n"))
        self.assertFalse(compare_tsv(b"2\n1\n", b"1\n2\n"))
        self.assertFalse(compare_tsv(b"1\t2\n", b"1\n"))
        self.assertFalse(compare_tsv(b"1", b"1\n"))
        self.assertFalse(compare_tsv(b"NULL\n", b"0\n"))

    def test_control_line_breaks_cannot_fake_matching_rows(self):
        self.assertFalse(compare_tsv(b"1\r2\n", b"1\n2\n"))
        self.assertFalse(compare_tsv(b"1\v2\n", b"1\n2\n"))
        self.assertTrue(compare_tsv(b"1\r\n2\r\n", b"1\n2\n"))

    def test_rejects_nan_infinity_and_bad_tolerances(self):
        self.assertFalse(numeric_equal(b"NaN", b"nan"))
        self.assertFalse(numeric_equal(b"NaN", b"NaN"))
        self.assertFalse(numeric_equal(b"Infinity", b"Infinity"))
        self.assertFalse(numeric_equal(b"-Infinity", b"-Infinity"))
        self.assertTrue(numeric_equal(b"NULL", b"NULL"))
        self.assertFalse(numeric_equal(b"Infinity", b"1"))
        for value in ("NaN", "Infinity", "-1", "abc"):
            with self.assertRaises(ValueError):
                parse_tolerance(value)


if __name__ == "__main__":
    unittest.main()

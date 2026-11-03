import unittest
from mismatch_diagnostics import describe_mismatch


class MismatchDiagnosticTests(unittest.TestCase):
    def test_reports_headers_and_counts(self):
        self.assertIn("header mismatch",
                      describe_mismatch(b"bad\n1\n", b"good\n1\n", "exact"))
        self.assertIn("expected 2, got 1",
                      describe_mismatch(b"id\n1\n", b"id\n1\n2\n", "exact"))

    def test_locates_field_without_exposing_values(self):
        summary = describe_mismatch(b"id\tx\n1\tsecret\n", b"id\tx\n1\tother\n", "exact")
        self.assertIn("row 1, column 2", summary)
        self.assertNotIn("secret", summary)
        self.assertNotIn("other", summary)

    def test_duplicate_multiplicity_is_visible(self):
        summary = describe_mismatch(b"x\na\na\n", b"x\na\nb\n", "unordered-header")
        self.assertIn("1 missing and 1 unexpected", summary)

    def test_numeric_mode_does_not_guess_first_inexact_cell(self):
        self.assertIn("numeric reference",
                      describe_mismatch(b"x\n1.002\n", b"x\n1.0\n", "numeric"))


if __name__ == "__main__":
    unittest.main()

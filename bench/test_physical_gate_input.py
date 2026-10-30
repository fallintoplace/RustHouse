"""Physical benchmark gates fail closed for malformed input."""
import unittest

from bench.scale.physical_gate import validate


class PhysicalGateInputTests(unittest.TestCase):
    def test_requires_report_object(self):
        for report in (None, [], "report", 42):
            with self.subTest(report=report), self.assertRaisesRegex(ValueError, "object"):
                validate(report)

    def test_requires_integer_row_floor(self):
        for rows in (True, False, 0, -1, 1.5, "1"):
            with self.subTest(rows=rows), self.assertRaisesRegex(ValueError, "positive integer"):
                validate({}, min_rows=rows)

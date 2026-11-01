"""Physical scans must verify an actual complete doctor success record."""
import unittest

from bench.scale.physical_scan import doctor_verified_rows


class PhysicalScanDoctorLineTests(unittest.TestCase):
    def test_accepts_complete_success_line(self):
        self.assertTrue(doctor_verified_rows(b"Database:\nOK events: 123 rows, 2 parts\n", 123))
        self.assertTrue(doctor_verified_rows(b"OK events: 1 rows, 1 parts\r\n", 1))

    def test_rejects_diagnostic_substrings_and_mismatches(self):
        for payload in (b"warning: OK events: 123 rows, 2 parts\n",
                        b"NOT OK events: 123 rows, 2 parts\n",
                        b"OK events: 123 rows, 0 parts\n",
                        b"OK events: 123 rows, 2 parts extra\n",
                        b"OK events: 124 rows, 2 parts\n"):
            with self.subTest(payload=payload):
                self.assertFalse(doctor_verified_rows(payload, 123))


if __name__ == "__main__":
    unittest.main()

import unittest
from doctor_report import report

class DoctorReportTests(unittest.TestCase):
    def test_success(self):
        result = report(0, "ok", "", 1.23456, True)
        self.assertTrue(result["ok"])
        self.assertEqual(result["elapsed_seconds"], 1.235)
        self.assertFalse(result["truncated"])

    def test_failure_and_bounded_output(self):
        result = report(1, "x" * 40000, "error", 0.1, False)
        self.assertFalse(result["ok"])
        self.assertEqual(len(result["stdout"]), 32768)
        self.assertTrue(result["truncated"])


if __name__ == "__main__":
    unittest.main()

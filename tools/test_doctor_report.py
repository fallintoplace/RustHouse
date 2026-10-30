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


    def test_runner_spools_large_doctor_output(self):
        from unittest.mock import patch
        from doctor_report import run
        def emit(command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            kwargs["stdout"].write(b"v" * 100_000)
            kwargs["stderr"].write(b"failure")
            return type("Completed", (), {"returncode": 1})()
        with patch("doctor_report.subprocess.run", side_effect=emit):
            item = run("rusthouse", "db", full_hashes=False)
        self.assertFalse(item["ok"])
        self.assertEqual(len(item["stdout"]), 32_768)
        self.assertEqual(item["stderr"], "failure")
        self.assertTrue(item["truncated"])

    def test_custom_doctor_timeout_is_forwarded(self):
        from unittest.mock import patch
        from doctor_report import run
        with patch("doctor_report.subprocess.run") as subprocess_run:
            subprocess_run.return_value.returncode = 0
            result = run("rusthouse", "db", timeout=12.5)
            self.assertTrue(result["ok"])
            self.assertEqual(subprocess_run.call_args.kwargs["timeout"], 12.5)
            with self.assertRaisesRegex(ValueError, "timeout"):
                run("rusthouse", "db", timeout=float("nan"))

if __name__ == "__main__":
    unittest.main()

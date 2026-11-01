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


    def test_utf8_multibyte_output_respects_byte_limit(self):
        result = report(0, "界" * 20_000, "🦀" * 20_000, 0.1, True)
        self.assertLessEqual(len(result["stdout"].encode("utf-8")), 32_768)
        self.assertLessEqual(len(result["stderr"].encode("utf-8")), 32_768)
        self.assertTrue(result["truncated"])
        self.assertNotIn("\ufffd", result["stdout"])
        self.assertNotIn("\ufffd", result["stderr"])

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

    def test_doctor_failure_retains_last_diagnostic(self):
        from unittest.mock import patch
        from doctor_report import run
        def fail(command, **kwargs):
            kwargs["stderr"].write(b"x" * 100_000 + b"FATAL: checksum mismatch")
            return type("Completed", (), {"returncode": 1})()
        with patch("doctor_report.subprocess.run", side_effect=fail):
            result = run("rusthouse", "db")
        self.assertIn("FATAL: checksum mismatch", result["stderr"])
        self.assertLessEqual(len(result["stderr"]), 32_768)
        self.assertTrue(result["truncated"])

    def test_subprocess_cannot_fill_temp_files_before_report_truncation(self):
        import os
        import sys
        import tempfile
        from pathlib import Path
        from doctor_report import MAX_CHILD_OUTPUT_BYTES, run

        if os.name != "posix":
            self.skipTest("RLIMIT_FSIZE is available on POSIX")
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "noisy-doctor"
            binary.write_text(
                "#!" + sys.executable + "\n"
                "import os\n"
                "for _ in range(256):\n"
                "    os.write(1, b'x' * 65536)\n"
            )
            binary.chmod(0o700)
            outcome = run(binary, directory, full_hashes=False, timeout=10)
        self.assertFalse(outcome["ok"])
        self.assertTrue(outcome["truncated"])
        self.assertEqual(len(outcome["stdout"]), 32_768)
        self.assertLess(MAX_CHILD_OUTPUT_BYTES, 256 * 65536)

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

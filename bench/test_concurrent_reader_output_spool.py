"""Concurrent physical reads verify bounded output without capturing it in RAM."""
import subprocess
import unittest
from unittest import mock

from bench.scale.concurrent_readers import run_readers


class ConcurrentReaderStreamingTests(unittest.TestCase):
    def test_live_sql_path_uses_spooled_output(self):
        def fake_run(cmd, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            self.assertEqual(kwargs["stderr"], subprocess.DEVNULL)
            kwargs["stdout"].write(b"ok\n")
            return subprocess.CompletedProcess(cmd, 0, b"", b"")

        with mock.patch("bench.scale.concurrent_readers.subprocess.run",
                        side_effect=fake_run) as execute:
            report = run_readers("rusthouse", "/parts", "SELECT count(*) FROM part",
                                 b"ok\n", clients=1, rounds=2)
            self.assertTrue(report["all_verified"])
            self.assertEqual(report["verified"], 2)
            self.assertEqual(execute.call_count, 2)

    def test_oversized_sql_output_fails_reference_comparison(self):
        def fake_run(cmd, **kwargs):
            kwargs["stdout"].write(b"ok\n" + b"x" * 100_000)
            return subprocess.CompletedProcess(cmd, 0, b"", b"")

        with mock.patch("bench.scale.concurrent_readers.subprocess.run",
                        side_effect=fake_run):
            report = run_readers("rusthouse", "/parts", "SELECT count(*) FROM part",
                                 b"ok\n", clients=1, rounds=1)
            self.assertFalse(report["all_verified"])
            self.assertEqual(report["verified"], 0)
            self.assertEqual(report["results"][0]["error"], "result differs from reference")

    def test_injected_runner_behavior_is_preserved(self):
        def fake_run(cmd, **kwargs):
            self.assertTrue(kwargs.get("capture_output"))
            return subprocess.CompletedProcess(cmd, 0, b"1\n", b"")

        report = run_readers("rusthouse", "/parts", "SELECT 1", b"1\n",
                             clients=1, rounds=1, execute=fake_run)
        self.assertTrue(report["all_verified"])

import unittest
from types import SimpleNamespace

from bench.scale.concurrent_readers import run_readers


class ConcurrentReadersTests(unittest.TestCase):
    def test_exact_reference_checked_for_every_reader(self):
        def run(command, **kwargs):
            return SimpleNamespace(returncode=0, stdout=b"2\n", stderr=b"")
        report = run_readers("rusthouse", "part", "SELECT count(*) FROM part",
                             b"2\n", clients=3, rounds=2, execute=run)
        self.assertEqual(report["planned"], 6)
        self.assertEqual(report["verified"], 6)
        self.assertTrue(report["all_verified"])
        self.assertGreater(report["verified_queries_per_second"], 0)

    def test_mismatch_is_not_success(self):
        result = run_readers("rusthouse", "part", "SELECT 1", b"1\n",
                             clients=2, rounds=1, execute=lambda *a, **k:
                             SimpleNamespace(returncode=0, stdout=b"2\n", stderr=b""))
        self.assertEqual(result["verified"], 0)
        self.assertFalse(result["all_verified"])

    def test_rejects_float_worker_counts_and_nonfinite_timeout(self):
        for kwargs in ({"clients": 1.0}, {"rounds": 2.5},
                       {"timeout": float("nan")}, {"timeout": float("inf")}):
            with self.subTest(kwargs=kwargs):
                with self.assertRaises(ValueError):
                    run_readers("binary", "part", "SELECT 1", b"1\n", **kwargs)

    def test_caps_total_jobs_and_output_bytes(self):
        with self.assertRaisesRegex(ValueError, "256"):
            run_readers("binary", "part", "SELECT 1", b"", clients=64, rounds=5)
        with self.assertRaisesRegex(ValueError, "1 MiB"):
            run_readers("binary", "part", "SELECT 1", b"x" * (1024 * 1024 + 1))


if __name__ == "__main__":
    unittest.main()

"""Fresh-process digest agreement catches nondeterministic query output."""
import hashlib
import unittest
from types import SimpleNamespace

from bench.scale.query_digest import verify_query_stability


class StableQueryDigestTests(unittest.TestCase):
    def test_exact_bytes_and_hash_agree_across_runs(self):
        calls = []
        def runner(command, **kwargs):
            calls.append(command)
            kwargs["stdout"].write(b"7\n8\n")
            return SimpleNamespace(returncode=0)
        report = verify_query_stability(
            "binary", "part", "SELECT id FROM t ORDER BY id",
            repetitions=5, run=runner)
        self.assertEqual(len(calls), 5)
        self.assertEqual(report["bytes"], 4)
        self.assertEqual(report["sha256"], hashlib.sha256(b"7\n8\n").hexdigest())
        self.assertEqual(report["stability_repetitions"], 5)
        self.assertTrue(report["output_stability_verified"])
        self.assertGreaterEqual(report["total_seconds"], report["maximum_seconds"])

    def test_detects_equal_length_different_content(self):
        runs = iter((b"1\n", b"2\n"))
        def runner(_command, **kwargs):
            kwargs["stdout"].write(next(runs))
            return SimpleNamespace(returncode=0)
        with self.assertRaisesRegex(ValueError, "changed on stability repetition 2"):
            verify_query_stability("binary", "part", "SELECT id FROM t",
                                   repetitions=2, run=runner)

    def test_rejects_invalid_repeat_values_before_running(self):
        for count in (True, 1, 17, 2.0, 0):
            with self.subTest(count=count), self.assertRaisesRegex(
                ValueError, "repetitions"
            ):
                verify_query_stability("binary", "part", "SELECT 1",
                                       repetitions=count)


if __name__ == "__main__":
    unittest.main()

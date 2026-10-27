import tempfile
import unittest
from pathlib import Path
from fixture import DATE_KEYS, generate
from run import SQL, benchmark, expected_rows


class StarJoinBenchmarkTests(unittest.TestCase):
    def test_reference_checked_execution(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=15, seed=8)
            expected = expected_rows(Path(root) / "reference.sqlite")
        seen = []
        def execute(sql):
            seen.append(sql)
            if sql == SQL:
                return expected
            if sql == "SELECT COUNT(*) FROM date_dim":
                return str(len(DATE_KEYS)).encode() + b"\n"
            if sql == "SELECT COUNT(*) FROM lineorder":
                return b"15\n"
            return b""
        result = benchmark("fake", rows=15, seed=8, runs=2, warmups=1, execute=execute)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(len(result["timings_seconds"]), 2)
        self.assertFalse(result["official_ssb"])
        self.assertEqual(sum(query.startswith("COPY ") for query in seen), 2)

    def test_reference_mismatch_and_input_limit(self):
        def execute(sql):
            if sql == "SELECT COUNT(*) FROM date_dim":
                return str(len(DATE_KEYS)).encode() + b"\n"
            if sql == "SELECT COUNT(*) FROM lineorder":
                return b"3\n"
            if sql == SQL:
                return b"wrong\n"
            return b""
        result = benchmark("fake", rows=3, warmups=0, execute=execute)
        self.assertEqual(result["status"], "mismatch")
        self.assertEqual(result["timings_seconds"], [])
        with self.assertRaisesRegex(ValueError, "invalid row"):
            benchmark("fake", rows=20000)


if __name__ == "__main__":
    unittest.main()

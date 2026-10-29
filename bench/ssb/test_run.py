import tempfile
import unittest
from pathlib import Path
from fixture import DATE_KEYS, generate
from run import COUNT_SQL, GROUPED_SQL, SQL, benchmark, expected_rows


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

    def test_large_join_count_and_grouped_revenue_reference_modes(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=40961, seed=8)
            expected = {
                query: expected_rows(Path(root) / "reference.sqlite", statement)
                for query, statement in (
                    ("count-join", COUNT_SQL), ("grouped-revenue", GROUPED_SQL)
                )
            }
        for query, statement in (
            ("count-join", COUNT_SQL), ("grouped-revenue", GROUPED_SQL)
        ):
            def execute(sql):
                if sql == statement:
                    return expected[query]
                if sql == "SELECT COUNT(*) FROM date_dim":
                    return str(len(DATE_KEYS)).encode() + b"\n"
                if sql == "SELECT COUNT(*) FROM lineorder":
                    return b"40961\n"
                return b""
            result = benchmark("fake", rows=40961, seed=8, runs=1,
                               warmups=0, execute=execute, query=query)
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["query"], query)
            self.assertEqual(len(result["timings_seconds"]), 1)

    def test_late_result_mismatch_clears_partial_timings(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=15, seed=8)
            reference = expected_rows(Path(root) / "reference.sqlite")
        calls = 0

        def execute(sql):
            nonlocal calls
            if sql == "SELECT COUNT(*) FROM date_dim":
                return str(len(DATE_KEYS)).encode() + b"\n"
            if sql == "SELECT COUNT(*) FROM lineorder":
                return b"15\n"
            if sql == SQL:
                calls += 1
                return reference if calls == 1 else b"wrong\n"
            return b""

        result = benchmark("fake", rows=15, seed=8, runs=3, warmups=0, execute=execute)
        self.assertEqual(result["status"], "mismatch")
        self.assertEqual(result["timings_seconds"], [])
        self.assertIsNone(result["latency"])

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
            benchmark("fake", rows=1_000_001)


class SSBParameterValidationTests(unittest.TestCase):
    def test_bad_controls_do_not_generate_large_fixtures(self):
        for options in ({"rows": True}, {"rows": 5.5},
                        {"runs": True}, {"warmups": False},
                        {"timeout": float("nan")}, {"timeout": float("inf")},
                        {"query": []}):
            with self.subTest(options=options):
                with self.assertRaisesRegex(ValueError, "invalid row count"):
                    benchmark("unused", **options)

if __name__ == "__main__":
    unittest.main()

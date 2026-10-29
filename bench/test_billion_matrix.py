"""Small, deterministic checks for billion-row physical query oracles."""
import unittest
from bench.scale.billion_matrix import benchmark, bound, compile_case, load_cases


class BillionMatrixTests(unittest.TestCase):
    def test_cases_generate_bounded_expected_output_at_billion_scale(self):
        cases = load_cases()
        self.assertGreaterEqual(len(cases), 1)
        for case in cases:
            sql, expected = compile_case(case, 1_000_000_000)
            self.assertTrue(sql.startswith("SELECT "))
            self.assertLessEqual(len(expected), 64 * 1024)
            self.assertTrue(expected.endswith(b"\n"))

    def test_boundaries_are_calculated_without_rows_in_memory(self):
        self.assertEqual(bound("tail1m", 1_000_000_000), 999_000_000)
        self.assertEqual(bound("p75", 1_000_000_000), 750_000_000)
        with self.assertRaisesRegex(ValueError, "unknown boundary"):
            bound("id; DROP TABLE events", 1_000_000_000)

    def test_exact_modulo_oracles_and_topk(self):
        count_sql, count = compile_case(
            {"id": "mod", "kind": "count_mod", "modulus": 97, "residue": 1}, 1000,
        )
        self.assertIn("id % 97 = 1", count_sql)
        self.assertEqual(count, b"11\n")
        sql, output = compile_case(
            {"id": "tail", "kind": "topk_desc", "lo": "p50", "hi": "end",
             "limit": 3, "offset": 1}, 100,
        )
        self.assertIn("DESC LIMIT 3 OFFSET 1", sql)
        self.assertEqual(output, b"98\n97\n96\n")

    def test_preflight_rejects_wrong_snapshot_count(self):
        calls = []

        def fake(binary, part, sql, timeout):
            calls.append(sql)
            return b"999\n", 0.1

        with self.assertRaisesRegex(ValueError, "snapshot row count"):
            benchmark("rusthouse", "parts", 1_000_000_000, execute=fake)
        self.assertEqual(calls, ["SELECT count(*) FROM part"])

    def test_benchmark_checks_every_sample(self):
        case = {"id": "count-small", "kind": "count_range",
                "lo": "p25", "hi": "p75"}
        sql, expected = compile_case(case, 100)
        calls = []

        def fake(binary, part, statement, timeout):
            calls.append(statement)
            if statement == "SELECT count(*) FROM part":
                return b"100\n", 0.2
            self.assertEqual(statement, sql)
            return expected, 0.1

        result = benchmark("rusthouse", "parts", 100, runs=2, warmups=1,
                           execute=fake, cases=[case])
        self.assertTrue(result["all_verified"])
        self.assertEqual(len(result["results"][0]["samples_seconds"]), 2)
        self.assertEqual(len(calls), 4)

    def test_rejects_unsafe_spec_and_nonfinite_timeout(self):
        with self.assertRaisesRegex(ValueError, "invalid case schema"):
            compile_case({"id": "bad", "kind": "count_range",
                          "lo": "start", "hi": "end", "sql": "DROP"}, 100)
        with self.assertRaisesRegex(ValueError, "timeout"):
            benchmark("rusthouse", "parts", 100, timeout=float("nan"), cases=[])


if __name__ == "__main__":
    unittest.main()

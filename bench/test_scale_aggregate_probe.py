import unittest
from types import SimpleNamespace

from bench.scale.aggregate_probe import aggregates, verify


class AggregateProbeTests(unittest.TestCase):
    def test_billion_row_count_and_sum_oracle(self):
        values = dict(aggregates(1_000_000_000))
        self.assertEqual(values["count(*)"], 1_000_000_000)
        self.assertEqual(values["sum(id)"], 499_999_999_500_000_000)

    def test_offset_column_uses_exact_arithmetic_series(self):
        start = (1 << 64) - 3
        expected = dict(aggregates(3, start=start, column="event_id"))
        self.assertEqual(expected["sum(event_id)"], 3 * start + 3)
        self.assertEqual(expected["min(event_id)"], start)
        self.assertEqual(expected["max(event_id)"], (1 << 64) - 1)
        def execute(command, **kwargs):
            expr = command[2].removeprefix("SELECT ").removesuffix(" FROM part")
            return SimpleNamespace(returncode=0,
                stdout=f"{expected[expr]}\n".encode(), stderr=b"")
        report = verify("rusthouse", "part", 3, start=start,
                        column="event_id", execute=execute)
        self.assertEqual(report["column"], "event_id")
        with self.assertRaisesRegex(ValueError, "interval"):
            aggregates(4, start=start)
        with self.assertRaisesRegex(ValueError, "identifier"):
            aggregates(4, column="id;DROP")

    def test_rejects_boolean_float_and_overflow_rows(self):
        for rows in (True, 1.0, -1, 0, 2**64):
            with self.subTest(rows=rows):
                with self.assertRaisesRegex(ValueError, "UInt64"):
                    aggregates(rows)

    def test_verifies_all_aggregates(self):
        expected = dict(aggregates(5))
        def execute(command, **kwargs):
            expression = command[2].removeprefix("SELECT ").removesuffix(" FROM part")
            return SimpleNamespace(returncode=0, stdout=f"{expected[expression]}\n".encode(), stderr=b"")
        report = verify("rusthouse", "parts", 5, execute=execute)
        self.assertEqual(len(report["results"]), 4)
        self.assertTrue(all(r["verified"] for r in report["results"]))

    def test_mismatch_rejected(self):
        with self.assertRaisesRegex(ValueError, "differs"):
            verify("rusthouse", "parts", 5, execute=lambda *a, **kw:
                   SimpleNamespace(returncode=0, stdout=b"wrong\n", stderr=b""))

    def test_rejects_nonfinite_query_timeout(self):
        from bench.scale.aggregate_probe import verify
        for timeout in (float("nan"), float("inf"), float("-inf"), True):
            with self.assertRaisesRegex(ValueError, "timeout"):
                verify("rusthouse", "part", 1, timeout=timeout)


if __name__ == "__main__":
    unittest.main()

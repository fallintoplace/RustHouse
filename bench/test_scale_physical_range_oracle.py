import re
import unittest

from bench.scale.physical_range_oracle import expected, sql_for, verify


class PhysicalRangeOracleTests(unittest.TestCase):
    def test_closed_form_at_billion_rows(self):
        self.assertEqual(expected(0, 1_000_000_000),
                         (1_000_000_000, 499_999_999_500_000_000))
        self.assertEqual(expected(2**64 - 2, 2),
                         (2, 2 * (2**64 - 2) + 1))
        with self.assertRaises(ValueError):
            expected(2**64 - 2, 3)

    def test_verifies_windows_and_fails_on_wrong_sum(self):
        def run(query):
            nums = [int(x) for x in re.findall(r"id >= (\d+)|id < (\d+)", query)
                    for x in x if x]
            # For this test every window has an upper-exclusive bound.
            first = nums[0]
            stop = nums[1]
            count, total = expected(first, stop - first)
            return f"{count}\t{total}\n".encode()
        result = verify("x", "x", "events", 10, 6, 2, execute=run)
        self.assertEqual(result["windows"], 3)
        with self.assertRaisesRegex(ValueError, "expected"):
            verify("x", "x", "events", 10, 2, 2,
                   execute=lambda _: b"2\t999\n")

    def test_custom_key_column_uses_safe_sql(self):
        statement = sql_for("events", 20, 3, column="event_id")
        self.assertIn("sum(event_id)", statement)
        self.assertIn("event_id >= 20", statement)
        self.assertIn("event_id < 23", statement)
        with self.assertRaisesRegex(ValueError, "column"):
            sql_for("events", 20, 3, column="id;DELETE")

    def test_rejects_noncanonical_numeric_wire_values(self):
        for raw in (b"+2\t21\n", b"02\t21\n", b"2\t21", b"2 \t21\n"):
            with self.subTest(raw=raw):
                with self.assertRaisesRegex(ValueError, "expected"):
                    verify("x", "x", "events", 10, 2, 2, execute=lambda _: raw)

    def test_rejects_unsafe_sql_names(self):
        with self.assertRaises(ValueError):
            sql_for("x;delete", 0, 1)

    def test_rejects_unbounded_billion_row_query_count(self):
        with self.assertRaisesRegex(ValueError, "10000 query windows"):
            verify("rusthouse", "db", "events", 0, 1_000_000_000,
                   window_rows=1, execute=lambda _: b"")


class PhysicalRangeTimeoutTypeTests(unittest.TestCase):
    def test_rejects_invalid_deadlines_before_running_queries(self):
        for timeout in (True, "20", float("nan"), float("inf")):
            with self.subTest(timeout=timeout):
                with self.assertRaisesRegex(ValueError, "finite"):
                    verify("unused", "db", "events", 10, 2, 2,
                           timeout=timeout, execute=lambda _: b"2\t21\n")

if __name__ == "__main__":
    unittest.main()

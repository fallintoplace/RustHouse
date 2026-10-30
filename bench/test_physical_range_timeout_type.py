"""Physical range oracle rejects invalid timing controls and keeps exact sums."""
import unittest

from bench.scale.physical_range_oracle import expected, verify


class PhysicalRangeOracleControlTests(unittest.TestCase):
    def test_rejects_boolean_or_text_timeouts_before_sql(self):
        def do_not_execute(_):
            self.fail("invalid settings reached a SQL query")

        for timeout in (True, False, "30"):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "timeout"):
                verify("unused", "db", "events", 0, 3, timeout=timeout,
                       execute=do_not_execute)
        with self.assertRaisesRegex(ValueError, "window_rows"):
            verify("unused", "db", "events", 0, 3, window_rows=True,
                   execute=do_not_execute)

    def test_multiple_windows_preserve_independent_sum_oracle(self):
        responses = iter((b"3\t9\n", b"2\t11\n"))
        result = verify("unused", "db", "events", 2, 5, window_rows=3,
                        timeout=30, execute=lambda _sql: next(responses))
        self.assertEqual(result["rows"], 5)
        self.assertEqual(result["sum"], expected(2, 5)[1])
        self.assertEqual(result["sum"], 20)
        self.assertEqual(result["windows"], 2)

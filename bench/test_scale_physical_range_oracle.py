import re
import unittest

from bench.scale.physical_range_oracle import expected, invoke, sql_for, verify


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

    def test_rejects_invalid_range_oracle_identifier_lengths(self):
        for table, column in (("x" * 129, "id"), ("events", "x" * 129), (None, "id")):
            with self.subTest(table=table, column=column), self.assertRaisesRegex(ValueError, "name"):
                sql_for(table, 0, 1, column=column)

    def test_rejects_unbounded_billion_row_query_count(self):
        with self.assertRaisesRegex(ValueError, "10000 query windows"):
            verify("rusthouse", "db", "events", 0, 1_000_000_000,
                   window_rows=1, execute=lambda _: b"")

    def test_subprocess_output_spooled_and_bounded(self):
        from types import SimpleNamespace
        from unittest.mock import patch

        def fake_run(_args, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            kwargs["stdout"].write(b"x" * 1025)
            return SimpleNamespace(returncode=0)

        with patch("bench.scale.physical_range_oracle.subprocess.run", side_effect=fake_run):
            with self.assertRaisesRegex(ValueError, "1 KiB"):
                invoke("rusthouse", "database", "SELECT 1", 30)

    def test_subprocess_failure_uses_bounded_error_tail(self):
        from types import SimpleNamespace
        from unittest.mock import patch

        def fake_run(_args, **kwargs):
            kwargs["stderr"].write(b"x" * 10000 + b"last error")
            return SimpleNamespace(returncode=1)

        with patch("bench.scale.physical_range_oracle.subprocess.run", side_effect=fake_run):
            with self.assertRaises(RuntimeError) as caught:
                invoke("rusthouse", "database", "SELECT 1", 30)
        self.assertIn("last error", str(caught.exception))
        self.assertLess(len(str(caught.exception)), 501)


    def test_default_window_runner_uses_remaining_total_deadline(self):
        from unittest.mock import patch
        with patch("bench.scale.physical_range_oracle.invoke",
                   return_value=b"2\t1\n") as invoke:
            with patch("bench.scale.physical_range_oracle.time.monotonic",
                       side_effect=[100.0, 100.2, 100.3]):
                report = verify("binary", "db", "events", 0, 2, window_rows=2,
                                timeout=30, max_total_seconds=1)
        self.assertEqual(report["windows"], 1)
        self.assertAlmostEqual(invoke.call_args.args[3], 0.8)

    def test_total_deadline_rejects_expired_windows_and_bad_inputs(self):
        from unittest.mock import patch
        for bad in (True, 0, -1, float("nan"), float("inf")):
            with self.subTest(value=bad), self.assertRaisesRegex(ValueError, "max_total_seconds"):
                verify("binary", "db", "events", 0, 1,
                       execute=lambda _: b"", max_total_seconds=bad)
        with patch("bench.scale.physical_range_oracle.time.monotonic",
                   side_effect=[0.0, 2.0]):
            with self.assertRaisesRegex(TimeoutError, "total deadline"):
                verify("binary", "db", "events", 0, 1,
                       execute=lambda _: b"1\t0\n", max_total_seconds=1)

    def test_tail_latency_is_bounded_by_maximum(self):
        def run(statement):
            import re
            first = int(re.search(r"id >= (\d+)", statement).group(1))
            last = int(re.search(r"id < (\d+)", statement).group(1))
            count, total = expected(first, last - first)
            return f"{count}\t{total}\n".encode()
        result = verify("binary", "db", "events", 0, 10, window_rows=2,
                        execute=run)
        self.assertGreaterEqual(result["p95_seconds"], result["median_seconds"])
        self.assertGreaterEqual(result["p99_seconds"], result["p95_seconds"])
        self.assertEqual(result["p99_seconds"], result["max_seconds"])

if __name__ == "__main__":
    unittest.main()

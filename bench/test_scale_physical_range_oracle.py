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

    def test_rejects_noncanonical_numeric_wire_values(self):
        for raw in (b"+2\t21\n", b"02\t21\n", b"2\t21", b"2 \t21\n"):
            with self.subTest(raw=raw):
                with self.assertRaisesRegex(ValueError, "expected"):
                    verify("x", "x", "events", 10, 2, 2, execute=lambda _: raw)

    def test_rejects_unsafe_sql_names(self):
        with self.assertRaises(ValueError):
            sql_for("x;delete", 0, 1)


if __name__ == "__main__":
    unittest.main()

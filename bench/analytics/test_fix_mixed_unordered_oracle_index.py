import unittest
from unittest.mock import patch

from bench.analytics import sqlite_oracle as oracle


class MixedUnorderedOracleIndexTests(unittest.TestCase):
    def test_indexed_mixed_group_results_match_in_linear_comparisons(self):
        expected = [(index, float(index) / 10) for index in range(2048)]
        output = "".join(f"{index}\t{value}\n" for index, value in reversed(expected)).encode()
        with patch.object(oracle, "row_equal", wraps=oracle.row_equal) as equal:
            oracle.assert_matches(output, expected, unordered=True)
        self.assertLessEqual(equal.call_count, 4096)

    def test_duplicate_exact_keys_still_respect_float_tolerance(self):
        expected = [(1, 0.1), (1, 0.2), (2, 0.3)]
        oracle.assert_matches(b"1\t0.2\n2\t0.3\n1\t0.1\n", expected, unordered=True)
        with self.assertRaisesRegex(ValueError, "multiset differs"):
            oracle.assert_matches(b"1\t0.2\n2\t0.3\n1\t0.5\n", expected, unordered=True)

    def test_null_text_and_numeric_keys_do_not_alias(self):
        expected = [(None, 1.25), (r"\\N", 2.5), (1, 3.75)]
        oracle.assert_matches(b"1\t3.75\n\\\\N\t2.5\n\\N\t1.25\n", expected, unordered=True)

    def test_all_float_columns_keep_tolerant_matching(self):
        expected = [(0.1,), (0.2,), (0.3,)]
        oracle.assert_matches(b"0.3\n0.1\n0.2\n", expected, unordered=True)
        with self.assertRaisesRegex(ValueError, "multiset differs"):
            oracle.assert_matches(b"0.3\n0.1\n0.8\n", expected, unordered=True)


if __name__ == "__main__":
    unittest.main()

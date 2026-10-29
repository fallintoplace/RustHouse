import unittest
from sf1_cardinality import SF1_ROWS, verify


class SF1CardinalityTests(unittest.TestCase):
    def test_all_eight_tables_match(self):
        lookup = {f"SELECT count(*) FROM {table}": str(n).encode()
                  for table, n in SF1_ROWS.items()}
        result = verify(None, None, execute=lambda sql: lookup[sql])
        self.assertEqual(result["tables_checked"], 8)
        self.assertTrue(result["cardinality_verified"])
        self.assertFalse(result["logical_data_verified"])

    def test_rejects_missing_or_truncated_data(self):
        def missing(sql):
            return b"4\n" if sql.endswith("region") else b"0\n"
        with self.assertRaisesRegex(ValueError, "region: SF1 expected 5"):
            verify(None, None, execute=missing)

    def test_rejects_ambiguous_non_numeric_count(self):
        for raw in (b"1.0\n", b"2\t3\n", b"-1\n", b"05\n", b"\n"):
            with self.subTest(raw=raw):
                with self.assertRaisesRegex(ValueError, "invalid count"):
                    verify(None, None, execute=lambda sql: raw)

    def test_invalid_timeout_rejected_before_executing(self):
        with self.assertRaisesRegex(ValueError, "timeout"):
            verify(None, None, timeout=0, execute=lambda _: b"5\n")


class SF1FiniteTimeoutTests(unittest.TestCase):
    def test_nonfinite_and_boolean_timeouts_are_rejected(self):
        for timeout in (float("nan"), float("inf"), True, "120"):
            with self.subTest(timeout=timeout):
                with self.assertRaisesRegex(ValueError, "finite"):
                    verify(None, None, timeout=timeout, execute=lambda _: b"5\n")

if __name__ == "__main__":
    unittest.main()

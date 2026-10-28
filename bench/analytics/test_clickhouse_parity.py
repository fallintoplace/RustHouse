"""Unit coverage without requiring a live ClickHouse installation."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bench.analytics.clickhouse_parity import (
    assert_clickhouse_parity, cli_clickhouse, clickhouse_create_table,
    fixture_json, reference_value,
)
from bench.analytics.suite_runner import ACCOUNTS_ROWS, COLUMNS, fact_row, make_fixture


class ClickHouseDifferentialTests(unittest.TestCase):
    def test_parity_accepts_floating_rounding_nulls_and_exact_big_integers(self):
        expected = b"9007199254740993\t1.3333333333333333\t\\N\n"
        actual = b"9007199254740993\t1.3333333333333335\tNULL\n"
        self.assertEqual(assert_clickhouse_parity(actual, expected), 1)
        with self.assertRaises(ValueError):
            assert_clickhouse_parity(
                b"9007199254740992\t1.3333333333333333\tNULL\n", expected
            )

    def test_unordered_results_preserve_duplicate_multiplicity(self):
        expected = b"west\t20\neast\t10\neast\t10\n"
        actual = b"east\t10\nwest\t20\neast\t10\n"
        self.assertEqual(assert_clickhouse_parity(actual, expected, unordered=True), 3)
        with self.assertRaises(ValueError):
            assert_clickhouse_parity(b"east\t10\nwest\t20\nwest\t20\n",
                                    expected, unordered=True)
        with self.assertRaises(ValueError):
            assert_clickhouse_parity(actual, expected, unordered=False)

    def test_value_parsing_does_not_lose_integer_precision(self):
        self.assertEqual(reference_value("18446744073709551615"), 18446744073709551615)
        self.assertEqual(reference_value("001"), "001")
        self.assertEqual(reference_value(r"\N"), None)
        self.assertEqual(reference_value("-10"), -10)
        self.assertEqual(reference_value("10.5"), 10.5)
        self.assertEqual(reference_value("EU"), "EU")

    def test_fixture_serialization_is_identical_to_rusthouse_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            _, oracle = make_fixture(directory, 137)
            try:
                events, accounts = fixture_json(directory, 137)
                lines = [json.loads(line) for line in events.read_text().splitlines()]
                self.assertEqual(len(lines), 137)
                for index in (0, 3, 136):
                    self.assertEqual(tuple(lines[index][column] for column in COLUMNS),
                                     fact_row(index))
                self.assertTrue(any(event["coupon"] is None for event in lines))
                self.assertEqual(len(accounts.read_text().splitlines()), ACCOUNTS_ROWS)
            finally:
                oracle.close()

    def test_clickhouse_uses_memory_engines_and_requires_create_table(self):
        schema = "CREATE TABLE events (id UInt64)"
        self.assertEqual(clickhouse_create_table(schema),
                         schema + " ENGINE = Memory")
        with self.assertRaises(ValueError):
            clickhouse_create_table("DROP TABLE events")

    def test_cli_propagates_failed_clickhouse_queries(self):
        with mock.patch("bench.analytics.clickhouse_parity.subprocess.run") as run:
            run.return_value = mock.Mock(returncode=23, stderr=b"invalid query", stdout=b"")
            with self.assertRaisesRegex(RuntimeError, "ClickHouse exit 23"):
                cli_clickhouse("clickhouse-client", "localhost", 9000,
                               "SELECT 1", 10)
            self.assertIn("--query", run.call_args.args[0])


if __name__ == "__main__":
    unittest.main()

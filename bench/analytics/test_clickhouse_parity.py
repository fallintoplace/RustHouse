"""Unit coverage without requiring a live ClickHouse installation."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bench.analytics.clickhouse_parity import (
    assert_clickhouse_parity, assert_clickhouse_json_parity, cli_clickhouse, clickhouse_create_table,
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


    def test_json_oracle_preserves_reference_types_and_large_integers(self):
        actual = (
            b'{"code":"001","id":"18446744073709551615","score":"1.3333333333333335","coupon":null}\n'
        )
        reference = (
            b'{"code":"001","id":18446744073709551615,"score":1.3333333333333333,"coupon":null}\n'
        )
        self.assertEqual(assert_clickhouse_json_parity(actual, reference), 1)
        for wrong in (
            actual.replace(b'"code":"001"', b'"code":"1"'),
            actual.replace(b'"coupon":null', b'"coupon":"NULL"'),
            actual.replace(b'18446744073709551615', b'18446744073709551614'),
            actual.replace(b'"code":"001"', b'"code":"0010"'),
        ):
            with self.subTest(wrong=wrong), self.assertRaisesRegex(ValueError, "differs"):
                assert_clickhouse_json_parity(wrong, reference)

    def test_json_oracle_preserves_unicode_escapes_and_string_backslashes(self):
        actual = '{"label":"\\\\N","detail":"café\\ttab"}\n'.encode()
        reference = '{"label":"\\\\N","detail":"café\\ttab"}\n'.encode()
        self.assertEqual(assert_clickhouse_json_parity(actual, reference), 1)
        with self.assertRaises(ValueError):
            assert_clickhouse_json_parity(actual, reference.replace(b'caf', b'baf'))

    def test_json_oracle_rejects_duplicate_columns_and_invalid_structure(self):
        for invalid in (
            b'{"id":"1","id":"1"}\n',
            b'["1"]\n',
            b'{"id":"1"}\n\n',
            b'{"id":NaN}\n',
            b'not json\n',
        ):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                assert_clickhouse_json_parity(invalid, b'{"id":1}\n')
        with self.assertRaisesRegex(ValueError, "row count"):
            assert_clickhouse_json_parity(b'{"id":"1"}\n', b'{"id":1}\n{"id":2}\n')

    def test_json_oracle_unordered_multiset_keeps_multiplicity(self):
        expected = b'{"region":"EU","n":1}\n{"region":"NA","n":2}\n{"region":"EU","n":1}\n'
        actual = b'{"region":"EU","n":"1"}\n{"region":"EU","n":"1"}\n{"region":"NA","n":"2"}\n'
        self.assertEqual(assert_clickhouse_json_parity(actual, expected, unordered=True), 3)
        with self.assertRaises(ValueError):
            assert_clickhouse_json_parity(actual, expected)
        with self.assertRaises(ValueError):
            assert_clickhouse_json_parity(actual.replace(b'"NA"', b'"EU"'),
                                         expected, unordered=True)

    def test_json_oracle_respects_columns_and_boolean_values(self):
        actual = b'{"ok":"1","value":"7"}\n'
        expected = b'{"ok":true,"value":7}\n'
        self.assertEqual(assert_clickhouse_json_parity(actual, expected), 1)
        with self.assertRaises(ValueError):
            assert_clickhouse_json_parity(b'{"value":"7","ok":"1"}\n', expected)
        with self.assertRaises(ValueError):
            assert_clickhouse_json_parity(b'{"ok":"true","value":"7"}\n', expected)

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


    def test_parity_json_is_derived_from_loaded_tsv_not_regenerated(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _, oracle = make_fixture(root, 5)
            oracle.close()
            path = root / "events.tsv"
            lines = path.read_text().splitlines()
            cells = lines[1].split("\t")
            cells[COLUMNS.index("amount")] = "777"
            lines[1] = "\t".join(cells)
            cells = lines[2].split("\t")
            cells[COLUMNS.index("coupon")] = r"\\N"
            lines[2] = "\t".join(cells)
            path.write_text("\n".join(lines) + "\n")
            accounts = root / "accounts.tsv"
            account_lines = accounts.read_text().splitlines()
            first_account = account_lines[1].split("\t")
            first_account[1] = r"new\tplan"
            account_lines[1] = "\t".join(first_account)
            accounts.write_text("\n".join(account_lines) + "\n")
            event_json, account_json = fixture_json(root, 5)
            rows = [json.loads(row) for row in event_json.read_text().splitlines()]
            self.assertEqual(rows[0]["amount"], 777)
            self.assertEqual(rows[1]["coupon"], r"\N")
            accounts = [json.loads(row) for row in account_json.read_text().splitlines()]
            self.assertEqual(accounts[0]["tier"], "new\tplan")

    def test_parity_fixture_rejects_corrupt_tsv_and_bad_numeric_types(self):
        for mutation in ("bad_header", "too_many_fields", "missing_row",
                         "null_in_nonnullable", "uint64_overflow", "unknown_escape", "oversized_row"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                _, oracle = make_fixture(root, 3)
                oracle.close()
                path = root / "events.tsv"
                lines = path.read_text().splitlines()
                if mutation == "bad_header":
                    lines[0] = lines[0].replace("event_id", "bad_id")
                if mutation == "too_many_fields":
                    lines[1] += "\textra"
                if mutation == "oversized_row":
                    lines[1] += "x" * (1024 * 1024)
                if mutation == "missing_row":
                    lines.pop()
                if mutation in ("null_in_nonnullable", "uint64_overflow", "unknown_escape"):
                    cells = lines[1].split("\t")
                    if mutation == "null_in_nonnullable":
                        cells[COLUMNS.index("amount")] = r"\N"
                    elif mutation == "uint64_overflow":
                        cells[COLUMNS.index("amount")] = str(1 << 64)
                    else:
                        cells[COLUMNS.index("region")] = r"\q"
                    lines[1] = "\t".join(cells)
                path.write_text("\n".join(lines) + "\n")
                with self.assertRaises(ValueError):
                    fixture_json(root, 3)

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


    def test_unordered_json_string_multisets_scale_and_preserve_duplicates(self):
        reference_rows = [
            {"region": f"r-{index % 100}", "coupon": None if index % 2 else "promo"}
            for index in range(3000)
        ]
        def encode(rows):
            return ("".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)).encode()
        reference = encode(reference_rows)
        actual = encode(reversed(reference_rows))
        self.assertEqual(assert_clickhouse_json_parity(actual, reference, unordered=True), 3000)
        wrong = encode(list(reference_rows[:-1]) + [{"region": "unexpected", "coupon": "promo"}])
        with self.assertRaisesRegex(ValueError, "multiset"):
            assert_clickhouse_json_parity(wrong, reference, unordered=True)

if __name__ == "__main__":
    unittest.main()

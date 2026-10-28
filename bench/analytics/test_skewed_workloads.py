"""Skewed analytics fixtures stress cardinality, NULL and heavy hitters."""
import collections
import tempfile
import unittest
from pathlib import Path

from bench.analytics.suite_runner import (
    COLUMNS, fact_row, load_workload, make_fixture, skewed_fact_row,
    sqlite_typed_reference,
)


class SkewedAnalyticsTests(unittest.TestCase):
    def test_hot_users_sparse_promos_and_revenue_outliers(self):
        records = [skewed_fact_row(i) for i in range(20_000)]
        users = collections.Counter(row[1] for row in records)
        regions = collections.Counter(row[2] for row in records)
        categories = collections.Counter(row[3] for row in records)
        self.assertGreater(len(users), 950)
        self.assertGreater(sum(count for _, count in users.most_common(8)) / len(records), 0.80)
        self.assertGreater(regions["NA"] / len(records), 0.70)
        self.assertGreater(categories["books"] / len(records), 0.65)
        self.assertGreater(sum(row[-1] is None for row in records) / len(records), 0.85)
        self.assertGreater(sum(row[7] == "paid" for row in records) / len(records), 0.68)
        self.assertGreater(sum(row[4] >= 1_000_000 for row in records), 50)
        self.assertEqual(skewed_fact_row(0), skewed_fact_row(0))
        self.assertEqual(skewed_fact_row(12_345), skewed_fact_row(12_345))

    def test_skewed_tsv_matches_sqlite_exactly(self):
        with tempfile.TemporaryDirectory() as directory:
            path, oracle = make_fixture(directory, 3001, distribution="skewed")
            try:
                raw = path.read_text().splitlines()
                self.assertEqual(raw[0].split("\t"), list(COLUMNS))
                self.assertEqual(len(raw), 3002)
                first = raw[1].split("\t")
                self.assertEqual(first[0], "1")
                self.assertEqual(first[1], "0")
                self.assertEqual(first[2], "NA")
                self.assertEqual(first[3], "books")
                self.assertEqual(first[-1], "promo")
                event_count, unique_users = oracle.execute(
                    "SELECT COUNT(*), COUNT(DISTINCT user_id) FROM events"
                ).fetchone()
                self.assertEqual(event_count, 3001)
                self.assertGreater(unique_users, 500)
                paid = oracle.execute(
                    "SELECT COUNT(*) FROM events WHERE status = 'paid'"
                ).fetchone()[0]
                self.assertGreater(paid, 2000)
            finally:
                oracle.close()

    def test_uniform_default_stays_byte_for_byte_compatible(self):
        with tempfile.TemporaryDirectory() as directory:
            path, oracle = make_fixture(directory, 41)
            try:
                self.assertEqual(
                    path.read_text().splitlines()[1].split("\t")[:4],
                    list(map(str, fact_row(0)[:4])),
                )
            finally:
                oracle.close()
        with self.assertRaises(ValueError):
            make_fixture("/unused", 3, distribution="invalid")

    def test_every_skewed_analytical_query_has_a_sqlite_result(self):
        path = Path(__file__).parent / "workloads" / "skewed-heavy-hitter-analytics.json"
        workload = load_workload(path)
        self.assertEqual(len(workload["queries"]), 12)
        with tempfile.TemporaryDirectory() as directory:
            _, oracle = make_fixture(directory, 1001, distribution="skewed")
            try:
                for case in workload["queries"]:
                    with self.subTest(query=case["name"]):
                        self.assertEqual(case["oracle"], "typed")
                        results = sqlite_typed_reference(oracle, case["sql"])
                        self.assertIsInstance(results, list)
                        self.assertTrue(results, case["name"])
            finally:
                oracle.close()


if __name__ == "__main__":
    unittest.main()

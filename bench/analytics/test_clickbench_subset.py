import tempfile
import unittest
from pathlib import Path
from fixture import generate
from sqlite_oracle import sqlite_query
from clickbench_subset import parse_ids, run_subset
from run import DEFAULT_QUERIES, load_queries


class VerifiedClickBenchTests(unittest.TestCase):
    def test_query_ids_and_range_validation(self):
        self.assertEqual(parse_ids("1-3,2,9"), [1, 2, 3, 9])
        with self.assertRaisesRegex(ValueError, "within"):
            parse_ids("10")

    def test_subset_requires_sqlite_verified_outputs(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, 300, 2)
            queries = load_queries(DEFAULT_QUERIES)
            def execute(sql):
                rows = sqlite_query(Path(root) / "hits.sqlite", sql)
                return "".join("\t".join(map(str, row)) + "\n" for row in rows).encode()
            result = run_subset("fake", "db", Path(root) / "hits.sqlite",
                                only="1,2", runs=2, warmups=0, execute=execute)
            self.assertEqual(result["verified_queries"], 2)
            self.assertEqual(result["attempted_queries"], 2)
            self.assertFalse(result["official_clickbench_score"])
            bad = run_subset("fake", "db", Path(root) / "hits.sqlite",
                             only="1", runs=1, warmups=0, execute=lambda sql: b"0\n")
            self.assertEqual(bad["verified_queries"], 0)
            self.assertEqual(bad["results"][0]["status"], "mismatch")


if __name__ == "__main__":
    unittest.main()

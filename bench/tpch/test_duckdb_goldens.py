import subprocess
import tempfile
import unittest
from pathlib import Path

from duckdb_goldens import generate


class DuckDBGoldensTests(unittest.TestCase):
    def make_queries(self, folder):
        root = Path(folder)
        for number in range(1, 23):
            (root / f"q{number:02d}.sql").write_text(
                f"SELECT {number};\n", encoding="utf-8"
            )

    def test_all_queries_export_unmodified_bytes(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            queries = root / "queries"
            queries.mkdir()
            self.make_queries(queries)
            seen = []
            def execute(args):
                seen.append(args)
                return subprocess.CompletedProcess(args, 0, b"1\t2\n", b"")
            report = generate("duckdb", root / "db.duckdb",
                              queries, root / "goldens", execute=execute)
            self.assertEqual(len(seen), 22)
            self.assertEqual(len(report["queries"]), 22)
            self.assertEqual((root / "goldens" / "q22.tsv").read_bytes(), b"1\t2\n")
            self.assertFalse(report["logical_input_equivalence_established"])

    def test_rejects_incomplete_corpus(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            (root / "q01.sql").write_text("SELECT 1")
            with self.assertRaisesRegex(ValueError, "full canonical"):
                generate("duckdb", "db", root, root / "goldens",
                         execute=lambda _: None)

    def test_failure_does_not_publish_partial_goldens(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            queries = root / "queries"
            queries.mkdir()
            self.make_queries(queries)
            calls = 0
            def fail_on_second(args):
                nonlocal calls
                calls += 1
                return subprocess.CompletedProcess(args, 1 if calls == 2 else 0,
                                                   b"ok\n", b"invalid query")
            with self.assertRaisesRegex(ValueError, "q02"):
                generate("duckdb", "db", queries, root / "goldens",
                         execute=fail_on_second)
            self.assertFalse((root / "goldens").exists())


if __name__ == "__main__":
    unittest.main()

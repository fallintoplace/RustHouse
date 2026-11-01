import tempfile
import unittest
from pathlib import Path

from bench.analytics.fixture import generate
from bench.analytics.load_fixture import load_fixture


class AnalyticsFixtureSqlContractTests(unittest.TestCase):
    def test_rejects_altered_create_before_any_sql_execution(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=2)
            path = Path(root) / "create.sql"
            path.write_text(path.read_text() + "DROP TABLE events;\n")
            calls = []
            with self.assertRaisesRegex(ValueError, "trusted generated"):
                load_fixture("rusthouse", "db", root, execute=lambda sql: calls.append(sql))
            self.assertEqual(calls, [])

    def test_rejects_copy_from_other_path_before_creating_table(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=2)
            (Path(root) / "copy.sql").write_text(
                "COPY hits FROM '/tmp/other.tsv' WITH (FORMAT TSV, HEADER TRUE);\n"
            )
            calls = []
            with self.assertRaisesRegex(ValueError, "trusted generated"):
                load_fixture("rusthouse", "db", root, execute=lambda sql: calls.append(sql))
            self.assertEqual(calls, [])

    def test_generated_statements_are_accepted(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=2)
            calls = []
            def execute(sql):
                calls.append(sql)
                return b"2" if sql == "SELECT COUNT(*) FROM hits" else b""
            result = load_fixture("rusthouse", "db", root, execute=execute)
            self.assertEqual(result["rows"], 2)
            self.assertEqual(len(calls), 3)


if __name__ == "__main__":
    unittest.main()

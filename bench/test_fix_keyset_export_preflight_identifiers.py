import tempfile
import unittest
from pathlib import Path
from bench.scale.keyset_export import export, page_sql

class KeysetIdentifierPreflightTests(unittest.TestCase):
    def test_invalid_identifiers_never_stage_export(self):
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root)
            target = folder / "ids.tsv"
            for table, column in (("events; DROP TABLE events", "id"),
                                  ("events", "id;DROP"),
                                  ("bad name", "id")):
                with self.subTest(table=table, column=column):
                    with self.assertRaisesRegex(ValueError, "identifiers"):
                        export("rusthouse", "db", table, target, 0, 2,
                               column=column, execute=lambda sql: self.fail("SQL executed"))
                    self.assertEqual(list(folder.iterdir()), [])

    def test_valid_identifiers_preserve_page_sql(self):
        self.assertIn("FROM events_2026", page_sql("events_2026", "id", 10, 2))

if __name__ == "__main__":
    unittest.main()

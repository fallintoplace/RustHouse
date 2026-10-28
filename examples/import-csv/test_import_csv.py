import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock
from import_csv import validate, sql_path, import_csv

class ImportTests(unittest.TestCase):
    def test_valid_data_and_bounded_commands(self):
        with tempfile.TemporaryDirectory() as directory:
            csv = Path(directory) / "events.csv"
            csv.write_text("id,region,amount\n1,EU,20\n2,US,30\n")
            self.assertEqual(validate(csv), 2)
            run = Mock()
            self.assertEqual(import_csv("rusthouse", directory, csv, "purchases", run), 2)
            self.assertEqual(run.call_count, 2)
            self.assertIn("WITH (FORMAT CSV, HEADER TRUE)", run.call_args.args[0][2])

    def test_sql_identifiers_are_checked(self):
        with self.assertRaises(ValueError):
            import_csv("rusthouse", "/tmp", Path("input.csv"), "x; DROP TABLE x")

    def test_file_paths_escape_quotes(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertIn("a''b.csv", sql_path(Path(directory) / "a'b.csv"))


if __name__ == "__main__":
    unittest.main()

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


    def test_copy_reads_the_exact_validated_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "events.csv"
            initial = "id,region,amount\n1,EU,20\n2,US,30\n"
            source.write_text(initial)
            statements = []

            def race(command, **kwargs):
                self.assertTrue(kwargs["check"])
                statement = command[2]
                statements.append(statement)
                if statement.startswith("CREATE"):
                    # Replacing the original after validation must not
                    # influence the bytes passed to the real COPY process.
                    source.write_text("id,region,amount\n99,BAD,1\n")
                else:
                    import re
                    match = re.search(r" FROM '([^']+)' WITH", statement)
                    self.assertIsNotNone(match)
                    staged = Path(match.group(1))
                    self.assertNotEqual(staged, source)
                    self.assertEqual(staged.read_text(), initial)

            self.assertEqual(import_csv("rusthouse", directory, source, "events", race), 2)
            self.assertEqual(len(statements), 2)
            self.assertEqual(source.read_text(), "id,region,amount\n99,BAD,1\n")

    def test_rejects_replaced_source_before_any_create_statement(self):
        import os
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "events.csv"
            other = Path(directory) / "other.csv"
            source.write_text("id,region,amount\n1,EU,2\n")
            other.write_text("id,region,amount\n9,US,8\n")
            real_open = os.open
            runner = Mock()

            def replace_during_open(path, flags):
                source.unlink()
                other.rename(source)
                return real_open(path, flags)

            with patch("import_csv.os.open", side_effect=replace_during_open):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    import_csv("rusthouse", directory, source, "events", runner)
            runner.assert_not_called()


if __name__ == "__main__":
    unittest.main()

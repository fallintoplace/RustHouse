import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.analytics import run


class AnalyticsSqlFileIdentityTests(unittest.TestCase):
    def test_rejects_sql_file_replaced_after_preflight(self):
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "queries.sql"
            substitute = Path(root) / "replacement.sql"
            target.write_text("SELECT 1\n", encoding="utf-8")
            substitute.write_text("SELECT 2\n", encoding="utf-8")
            real_open = os.open

            def swap_before_open(path, flags):
                substitute.replace(target)
                return real_open(path, flags)

            with patch.object(run.os, "open", side_effect=swap_before_open):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    run.load_queries(target)

    def test_rejects_sql_symlink_substitution(self):
        if not hasattr(os, "O_NOFOLLOW"):
            self.skipTest("platform does not support no-follow open")
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "queries.sql"
            substitute = Path(root) / "replacement.sql"
            target.write_text("SELECT 1\n", encoding="utf-8")
            substitute.write_text("SELECT 2\n", encoding="utf-8")
            real_open = os.open

            def swap_to_symlink(path, flags):
                target.unlink()
                target.symlink_to(substitute)
                return real_open(path, flags)

            with patch.object(run.os, "open", side_effect=swap_to_symlink):
                with self.assertRaises((OSError, ValueError)):
                    run.load_queries(target)

    def test_regular_sql_input_is_parsed_unchanged(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "queries.sql"
            path.write_text("-- comment\nSELECT 1;\nSELECT 2\n", encoding="utf-8")
            self.assertEqual(run.load_queries(path), ["SELECT 1", "SELECT 2"])


if __name__ == "__main__":
    unittest.main()

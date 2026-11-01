"""The keyset CLI handles its total deadline without a traceback."""
import contextlib
import io
import unittest
from unittest.mock import patch

from bench.scale import keyset_export


class KeysetExportDeadlineCliTests(unittest.TestCase):
    def test_total_deadline_fails_cleanly(self):
        stderr = io.StringIO()
        argv = ["--binary", "bin", "--database", "db", "--table", "events",
                "--rows", "1", "--output", "ids.tsv"]
        with patch.object(keyset_export, "export",
                          side_effect=TimeoutError("total export deadline exceeded")):
            with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit) as raised:
                keyset_export.main(argv)
        self.assertEqual(raised.exception.code, 1)
        self.assertIn("total export deadline exceeded", stderr.getvalue())
        self.assertNotIn("Traceback", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()

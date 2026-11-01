"""SSB fixture SQL file reads must reject path replacement."""
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import run


class SsbFixtureFileIdentityTests(unittest.TestCase):
    def test_reads_regular_schema_file(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "schema.sql"
            source.write_bytes(b"CREATE TABLE example (id UInt64);\n")
            self.assertEqual(run.read_fixture_sql(source),
                             "CREATE TABLE example (id UInt64)")

    def test_rejects_swap_before_open(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "schema.sql"
            other = Path(root) / "other.sql"
            source.write_bytes(b"CREATE TABLE good (id UInt64);\n")
            other.write_bytes(b"CREATE TABLE bad (id UInt64);\n")
            original = os.open
            def swap(*args, **kwargs):
                os.replace(other, source)
                return original(*args, **kwargs)
            with patch.object(run.os, "open", side_effect=swap):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    run.read_fixture_sql(source)


if __name__ == "__main__":
    unittest.main()

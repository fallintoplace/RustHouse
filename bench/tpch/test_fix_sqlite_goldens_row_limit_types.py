import sqlite3
import tempfile
import unittest
from pathlib import Path
from sqlite_goldens import write_one, generate

class StrictRowLimitTests(unittest.TestCase):
    def test_rejects_boolean_and_fractional_limits_before_writing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with sqlite3.connect(":memory:") as db:
                for limit in (True, 1.5, 0):
                    with self.subTest(limit=limit):
                        with self.assertRaisesRegex(ValueError, "max_rows"):
                            write_one(db, "SELECT 1", root / "golden.tsv", limit)
                        with self.assertRaisesRegex(ValueError, "max_rows"):
                            generate(root / "missing.db", root / "queries", root / "out", max_rows=limit)
                        self.assertFalse((root / "out").exists())

if __name__ == "__main__":
    unittest.main()

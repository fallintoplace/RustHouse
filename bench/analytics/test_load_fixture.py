import tempfile
import unittest
from pathlib import Path
from fixture import generate
from load_fixture import load_fixture


class ClickBenchLoadTests(unittest.TestCase):
    def test_requires_sqlite_and_rusthouse_counts_to_match(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=12, seed=3)
            commands = []
            def execute(sql):
                commands.append(sql)
                return b"12\n" if sql.startswith("SELECT COUNT") else b""
            result = load_fixture(Path("fake"), Path(root) / "db", root, execute=execute)
            self.assertEqual(result["rows"], 12)
            self.assertEqual(len(commands), 3)
            self.assertFalse(result["official_clickbench"])

    def test_detects_wrong_engine_count(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=12, seed=3)
            with self.assertRaisesRegex(ValueError, "row count"):
                load_fixture(Path("fake"), Path(root) / "db", root,
                             execute=lambda sql: b"1\n")


if __name__ == "__main__":
    unittest.main()

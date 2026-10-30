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

    def test_rejects_malformed_tsv_before_running_sql(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=12, seed=3)
            fixture = Path(root) / "hits.tsv"
            with fixture.open("a", encoding="utf-8") as output:
                output.write("bad\trow\n")
            called = []
            with self.assertRaisesRegex(ValueError, "invalid column count"):
                load_fixture(Path("fake"), Path(root) / "db", root,
                             execute=lambda sql: called.append(sql))
            self.assertEqual(called, [])

    def test_rejects_same_count_with_corrupted_tsv_value(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=12, seed=3)
            fixture = Path(root) / "hits.tsv"
            lines = fixture.read_text().splitlines()
            fields = lines[1].split("\t")
            fields[3] = "tampered"
            lines[1] = "\t".join(fields)
            fixture.write_text("\n".join(lines) + "\n")
            called = []
            with self.assertRaisesRegex(ValueError, "differs from SQLite"):
                load_fixture(Path("fake"), Path(root) / "db", root,
                             execute=lambda sql: called.append(sql))
            self.assertEqual(called, [])

    def test_detects_wrong_engine_count(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=12, seed=3)
            with self.assertRaisesRegex(ValueError, "row count"):
                load_fixture(Path("fake"), Path(root) / "db", root,
                             execute=lambda sql: b"1\n")


class ClickBenchLoadTimeoutTests(unittest.TestCase):
    def test_rejects_invalid_timeouts_before_file_access(self):
        for timeout in (float("nan"), float("inf"), 0, True, "30"):
            with self.subTest(timeout=timeout):
                with self.assertRaisesRegex(ValueError, "finite"):
                    load_fixture("unused", "db", "no-such-fixture",
                                 timeout=timeout, execute=lambda sql: b"")

if __name__ == "__main__":
    unittest.main()

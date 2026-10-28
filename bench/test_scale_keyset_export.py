import tempfile
import unittest
from pathlib import Path

from bench.scale.keyset_export import export, page_sql


class KeysetExportTests(unittest.TestCase):
    def test_export_verifies_each_page_and_sha(self):
        with tempfile.TemporaryDirectory() as temp:
            seen = []
            def run(sql):
                seen.append(sql)
                start = {1: 5, 2: 7, 3: 9}[len(seen)]
                end = min(start + 2, 10)
                return b"".join(f"{number}\n".encode() for number in range(start, end))
            path = Path(temp) / "out.tsv"
            report = export("unused", "unused", "events", path, 5, 5, 2,
                            execute=run)
            self.assertEqual(path.read_bytes(), b"5\n6\n7\n8\n9\n")
            self.assertEqual(report["pages"], 3)
            self.assertEqual(len(report["sha256"]), 64)
            self.assertIn("LIMIT 3", seen[0])

    def test_detects_duplicates_and_leaves_no_visible_file(self):
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp) / "out.tsv"
            with self.assertRaisesRegex(ValueError, "duplicated|unordered"):
                export("unused", "unused", "events", dest, 0, 2,
                       execute=lambda sql: b"0\n0\n")
            self.assertFalse(dest.exists())

    def test_rejects_identifier_injection_and_overflow(self):
        with self.assertRaises(ValueError):
            page_sql("events; DROP TABLE", "id", 0, 5)
        with self.assertRaises(ValueError):
            page_sql("events", "id", 2**64 - 1, 2)
        self.assertIn("<= 18446744073709551615",
                      page_sql("events", "id", 2**64 - 1, 1))


if __name__ == "__main__":
    unittest.main()

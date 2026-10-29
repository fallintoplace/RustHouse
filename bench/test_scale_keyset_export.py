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

    def test_batched_page_verification_handles_large_uint64_pages(self):
        with tempfile.TemporaryDirectory() as temp:
            start = 2**64 - 50000
            rows = 50000
            path = Path(temp) / "large.tsv"
            expected = "".join(f"{number}\n"
                               for number in range(start, start + rows)).encode("ascii")
            report = export("unused", "unused", "events", path, start, rows,
                            page_rows=50000, execute=lambda _: expected)
            self.assertEqual(path.read_bytes(), expected)
            self.assertEqual(report["pages"], 1)

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

    def test_rejects_overrun_total_export_deadline_before_copying(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "ids.tsv"
            with patch("bench.scale.keyset_export.time.monotonic", side_effect=[0.0, 2.0]):
                with self.assertRaisesRegex(TimeoutError, "total export deadline"):
                    export("unused", "unused", "events", output, 0, 5,
                           max_total_seconds=1, execute=lambda sql: b"0\n")
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()

"""Keyset export staging cannot remove someone else's pending file."""
import os
import tempfile
import unittest
from pathlib import Path

from bench.scale.keyset_export import export


class KeysetExportTempfileTests(unittest.TestCase):
    def test_existing_legacy_pending_file_is_preserved(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            output = root / "ids.tsv"
            previous = root / f".ids.tsv.{os.getpid()}.pending"
            previous.write_bytes(b"existing run")
            pages = iter((b"0\n1\n", b"2\n3\n"))
            report = export("rusthouse", "db", "events", output, 0, 4,
                            page_rows=2, execute=lambda _: next(pages))
            self.assertEqual(report["rows"], 4)
            self.assertEqual(output.read_bytes(), b"0\n1\n2\n3\n")
            self.assertEqual(previous.read_bytes(), b"existing run")

    def test_hash_mismatch_removes_only_the_new_stage(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            output = root / "ids.tsv"
            with self.assertRaisesRegex(ValueError, "SHA256"):
                export("rusthouse", "db", "events", output, 0, 1,
                       execute=lambda _: b"0\n", expected_sha256="0" * 64)
            self.assertFalse(output.exists())
            self.assertEqual(list(root.iterdir()), [])

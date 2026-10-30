import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from bench.scale.chunk_copy import copy_sql, load_chunks


class ChunkCopyTests(unittest.TestCase):
    def test_escapes_path_and_rejects_identifier_injection(self):
        self.assertIn("WITH (FORMAT TSV)", copy_sql("events", "events.tsv"))
        with self.assertRaisesRegex(ValueError, "identifier"):
            copy_sql("events; DROP TABLE users", "data.tsv")

    def test_loads_ordered_chunks(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            for name in ("part-000000.tsv", "part-000001.tsv"):
                (path / name).write_bytes(b"1\n")
            digest = hashlib.sha256(b"1\n").hexdigest()
            shards = [
                {"path": name, "rows": 1, "bytes": 2, "sha256": digest}
                for name in ("part-000000.tsv", "part-000001.tsv")
            ]
            (path / "manifest.json").write_text(json.dumps({
                "format": "rusthouse-tsv-shards-v1", "header": False,
                "total_rows": 2, "total_bytes": 4, "shards": shards,
            }))
            commands = []
            def execute(command, **kwargs):
                commands.append(command)
                return SimpleNamespace(returncode=0, stderr=b"")
            report = load_chunks("rusthouse", "db", "events", path, execute=execute)
            self.assertEqual(report["rows_reported_by_manifest"], 2)
            self.assertEqual(len(commands), 2)
            self.assertIn("part-000000.tsv", commands[0][2])

    def test_dry_run_checks_manifest_without_copying(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            payload = b"42\n"
            (directory / "part-000000.tsv").write_bytes(payload)
            (directory / "manifest.json").write_text(json.dumps({
                "format": "rusthouse-tsv-shards-v1", "header": False,
                "total_rows": 1, "total_bytes": len(payload),
                "shards": [{"path": "part-000000.tsv", "rows": 1,
                            "bytes": len(payload),
                            "sha256": hashlib.sha256(payload).hexdigest()}]}))
            report = load_chunks("no-binary", "no-db", "events", directory,
                                 dry_run=True, execute=lambda *args, **kwargs:
                                 self.fail("dry-run must never execute SQL"))
            self.assertEqual(report["shards_planned"], 1)
            self.assertEqual(report["shards_loaded"], 0)
            self.assertTrue(report["preflight_verified"])
            (directory / "part-000000.tsv").write_bytes(b"bad\n")
            with self.assertRaisesRegex(ValueError, "mismatch"):
                load_chunks("x", "x", "events", directory, dry_run=True)

    def test_corruption_fails_before_any_copy(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "part-000000.tsv").write_bytes(b"tampered\n")
            (path / "manifest.json").write_text(json.dumps({
                "format": "rusthouse-tsv-shards-v1", "header": False,
                "total_rows": 1, "total_bytes": 2,
                "shards": [{"path": "part-000000.tsv", "rows": 1, "bytes": 2,
                            "sha256": hashlib.sha256(b"1\n").hexdigest()}],
            }))
            calls = []
            with self.assertRaisesRegex(ValueError, "mismatch"):
                load_chunks("rusthouse", "db", "events", path,
                            execute=lambda *a, **k: calls.append(a))
            self.assertEqual(calls, [])

    def test_fails_closed_on_bad_manifest(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            path.write_text(json.dumps({"format": "unknown", "shards": []}))
            with self.assertRaisesRegex(ValueError, "headerless"):
                load_chunks("rusthouse", "db", "events", root)

    def test_rejects_excess_shards_before_copy(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "manifest.json").write_text(json.dumps({
                "format": "rusthouse-tsv-shards-v1", "header": False,
                "shards": [{"path": "a"}, {"path": "b"}],
            }))
            with patch("bench.scale.chunk_copy.MAX_SHARDS", 1):
                with self.assertRaisesRegex(ValueError, "shard limit"):
                    load_chunks("rusthouse", "db", "events", path)


    def test_rejects_invalid_copy_options_before_loading_manifest(self):
        for timeout in (0, -1, float("nan"), float("inf"), True, "1"):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "timeout"):
                load_chunks("bin", "db", "events", "/nonexistent", timeout=timeout)
        with self.assertRaisesRegex(ValueError, "dry_run"):
            load_chunks("bin", "db", "events", "/nonexistent", dry_run=1)

    def test_rejects_oversized_manifest_before_copy(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            path.write_bytes(b"x" * 64)
            with patch("bench.scale.chunk_copy.MAX_MANIFEST_BYTES", 16):
                with self.assertRaisesRegex(ValueError, "manifest exceeds"):
                    load_chunks("bin", "db", "events", root,
                                execute=lambda *a, **kw: self.fail("must not copy"))

if __name__ == "__main__":
    unittest.main()

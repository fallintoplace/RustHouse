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
                (path / name).write_text("1\n")
            (path / "manifest.json").write_text(json.dumps({
                "format": "rusthouse-tsv-shards-v1", "header": False,
                "shards": [{"path": "part-000000.tsv", "rows": 1},
                           {"path": "part-000001.tsv", "rows": 1}]
            }))
            commands = []
            def execute(command, **kwargs):
                commands.append(command)
                return SimpleNamespace(returncode=0, stderr=b"")
            report = load_chunks("rusthouse", "db", "events", path, execute=execute)
            self.assertEqual(report["rows_reported_by_manifest"], 2)
            self.assertEqual(len(commands), 2)
            self.assertIn("part-000000.tsv", commands[0][2])

    def test_fails_closed_on_bad_manifest(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            path.write_text(json.dumps({"format": "unknown", "shards": []}))
            with self.assertRaisesRegex(ValueError, "headerless"):
                load_chunks("rusthouse", "db", "events", root)


if __name__ == "__main__":
    unittest.main()

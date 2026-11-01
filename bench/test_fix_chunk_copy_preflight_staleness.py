import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from bench.scale.chunk_copy import load_chunks


class ChunkCopyStalePreflightTests(unittest.TestCase):
    def test_later_shard_changed_after_preflight_cannot_be_copied(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root)
            shards = []
            for index, payload in enumerate((b"1\n", b"2\n")):
                name = f"part-{index:06d}.tsv"
                (source / name).write_bytes(payload)
                shards.append({"path": name, "rows": 1, "bytes": len(payload),
                               "sha256": hashlib.sha256(payload).hexdigest()})
            (source / "manifest.json").write_text(json.dumps({
                "format": "rusthouse-tsv-shards-v1", "header": False,
                "total_rows": 2, "total_bytes": 4, "shards": shards,
            }))
            calls = []
            def execute(command, **kwargs):
                calls.append(command)
                if len(calls) == 1:
                    (source / "part-000001.tsv").write_bytes(b"999\n")
                return SimpleNamespace(returncode=0, stderr=b"")
            with self.assertRaisesRegex(ValueError, "changed after preflight"):
                load_chunks("binary", "db", "events", source, execute=execute)
            self.assertEqual(len(calls), 1)


if __name__ == "__main__":
    unittest.main()

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from bench.scale.chunk_copy import load_chunks


class DuplicateShardTests(unittest.TestCase):
    def test_rejects_repeated_shard_before_copy(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "part-000000.tsv").write_bytes(b"1\n")
            entry = {"path": "part-000000.tsv", "rows": 1, "bytes": 2,
                     "sha256": hashlib.sha256(b"1\n").hexdigest()}
            (root / "manifest.json").write_text(json.dumps({
                "format": "rusthouse-tsv-shards-v1", "header": False,
                "total_rows": 2, "total_bytes": 4,
                "shards": [entry, dict(entry)],
            }))
            commands = []
            with self.assertRaisesRegex(ValueError, "duplicate shard"):
                load_chunks("rusthouse", root / "db", "events", root,
                            execute=lambda *args, **kwargs: commands.append(args))
            self.assertEqual(commands, [])

    def test_rejects_nonobject_shard(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "manifest.json").write_text(json.dumps({
                "format": "rusthouse-tsv-shards-v1", "header": False,
                "shards": [None],
            }))
            with self.assertRaisesRegex(ValueError, "manifest entry"):
                load_chunks("rusthouse", root / "db", "events", root)


if __name__ == "__main__":
    unittest.main()

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from bench.scale.verify_id_shards import verify


class ManifestRelativeShardTests(unittest.TestCase):
    def test_relative_paths_are_resolved_from_manifest_directory(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            shard = root / "rows.tsv"
            shard.write_bytes(b"0\n1\n")
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({
                "schema_version": 1, "start": 0, "rows": 2,
                "shards": [{"path": "rows.tsv", "start": 0, "rows": 2,
                            "sha256": hashlib.sha256(shard.read_bytes()).hexdigest()}],
            }))
            self.assertEqual(verify(manifest), {"verified": True, "rows": 2, "shards": 1})


if __name__ == "__main__":
    unittest.main()

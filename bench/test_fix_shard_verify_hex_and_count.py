import json
import tempfile
import unittest
from pathlib import Path
from bench.scale.verify_id_shards import verify

class ManifestBoundsTests(unittest.TestCase):
    def test_rejects_invalid_hash_before_opening_shard(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "manifest.json"
            path.write_text(json.dumps({"schema_version": 1, "start": 0, "rows": 1,
                "shards": [{"path": "not-here.tsv", "start": 0, "rows": 1,
                            "sha256": "z" * 64}]}))
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                verify(path)

    def test_rejects_oversized_manifest_before_file_access(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "manifest.json"
            path.write_text(json.dumps({"schema_version": 1, "start": 0, "rows": 1,
                "shards": [{}] * 16385}))
            with self.assertRaisesRegex(ValueError, "16384"):
                verify(path)

if __name__ == "__main__":
    unittest.main()

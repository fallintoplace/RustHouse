"""Reject JSON boolean schema versions even when their numeric value matches."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from bench.scale.verify_id_shards import verify


class TypedShardSchemaVersionTests(unittest.TestCase):
    def test_boolean_version_does_not_bypass_manifest_validation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shard = root / "part.tsv"
            shard.write_bytes(b"0\n1\n")
            manifest = {
                "schema_version": 1,
                "start": 0,
                "rows": 2,
                "shards": [{
                    "start": 0, "rows": 2, "path": shard.name,
                    "sha256": hashlib.sha256(shard.read_bytes()).hexdigest(),
                }],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest))
            self.assertTrue(verify(manifest_path)["verified"])
            for version in (True, False, 1.0, "1", None):
                with self.subTest(version=version):
                    manifest["schema_version"] = version
                    manifest_path.write_text(json.dumps(manifest))
                    with self.assertRaisesRegex(ValueError, "version"):
                        verify(manifest_path)


if __name__ == "__main__":
    unittest.main()

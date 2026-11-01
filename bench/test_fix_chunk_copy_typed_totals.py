"""Manifest-wide COPY counters must use JSON integers, not numeric lookalikes."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from bench.scale.chunk_copy import load_chunks


class ChunkManifestTotalsTests(unittest.TestCase):
    def test_rejects_boolean_and_fractional_manifest_totals(self):
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            data = b"0\n"
            (directory / "part-000000.tsv").write_bytes(data)
            manifest = {
                "format": "rusthouse-tsv-shards-v1", "header": False,
                "total_rows": 1, "total_bytes": len(data),
                "shards": [{
                    "path": "part-000000.tsv", "rows": 1, "bytes": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(),
                }],
            }
            manifest_path = directory / "manifest.json"
            manifest_path.write_text(json.dumps(manifest))
            self.assertTrue(load_chunks("unused", "unused", "events", directory,
                                        dry_run=True)["preflight_verified"])
            for key, invalid in (("total_rows", True), ("total_rows", 1.0),
                                 ("total_bytes", 2.0), ("total_bytes", "2")):
                with self.subTest(key=key, invalid=invalid):
                    broken = dict(manifest)
                    broken[key] = invalid
                    manifest_path.write_text(json.dumps(broken))
                    with self.assertRaisesRegex(ValueError, "total"):
                        load_chunks("unused", "unused", "events", directory, dry_run=True)


if __name__ == "__main__":
    unittest.main()

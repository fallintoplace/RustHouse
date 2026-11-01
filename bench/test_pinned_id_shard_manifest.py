"""Verify shard manifests from unchanged, strict JSON files."""
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale import verify_id_shards


class ShardManifestTests(unittest.TestCase):
    def test_manifest_replacement_during_open_is_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            replacement = Path(root) / "other.json"
            path.write_text("{}", encoding="utf-8")
            replacement.write_text("{}", encoding="utf-8")
            original_open = os.open
            def swap(*args, **kwargs):
                os.replace(replacement, path)
                return original_open(*args, **kwargs)
            with patch.object(verify_id_shards.os, "open", side_effect=swap):
                with self.assertRaisesRegex(ValueError, "changed before opening"):
                    verify_id_shards.verify(path)

    def test_nonfinite_json_is_rejected_before_shard_access(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            path.write_text('{"schema_version":1,"start":0,"rows":1,"step":1,'
                            '"shards":[],"unexpected":NaN}', encoding="ascii")
            with self.assertRaisesRegex(ValueError, "JSON constant"):
                verify_id_shards.verify(path)


if __name__ == "__main__":
    unittest.main()

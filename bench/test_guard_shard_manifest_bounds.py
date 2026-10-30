import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.verify_id_shards import verify


class ShardManifestBoundsTests(unittest.TestCase):
    def test_rejects_excessive_manifest_bytes_before_json_parse(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            path.write_bytes(b"x" * 128)
            with patch("bench.scale.verify_id_shards.MAX_MANIFEST_BYTES", 32):
                with self.assertRaisesRegex(ValueError, "manifest exceeds"):
                    verify(path)

    @unittest.skipIf(sys.platform == "win32", "Windows CI may disallow symlinks")
    def test_refuses_to_follow_manifest_symlink(self):
        with tempfile.TemporaryDirectory() as root:
            real = Path(root) / "real.json"
            real.write_text('{"schema_version": 0}', encoding="utf-8")
            link = Path(root) / "linked.json"
            link.symlink_to(real)
            with self.assertRaisesRegex(ValueError, "symlinked shard manifest"):
                verify(link)


if __name__ == "__main__":
    unittest.main()

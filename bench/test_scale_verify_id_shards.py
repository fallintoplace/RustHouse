import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from bench.scale.verify_id_shards import verify, verify_file


class VerifyShardsTests(unittest.TestCase):
    def test_verifies_exact_large_integer_boundaries(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            start = 2**64 - 3
            source = root / "last.tsv"
            source.write_bytes(f"{start}\n{start + 1}\n{start + 2}\n".encode("ascii"))
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({
                "schema_version": 1, "start": start, "rows": 3,
                "shards": [{"path": str(source), "start": start, "rows": 3,
                            "sha256": hashlib.sha256(source.read_bytes()).hexdigest()}],
            }))
            self.assertEqual(verify(manifest)["rows"], 3)
            source.write_bytes(f"{start}\n{start + 2}\n{start + 1}\n".encode("ascii"))
            with self.assertRaisesRegex(ValueError, "incorrect"):
                verify(manifest)

    def test_rejects_overlap_and_incomplete_coverage(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "rows.tsv"
            source.write_bytes(b"0\n1\n")
            shard = {"path": str(source), "start": 0, "rows": 2,
                     "sha256": hashlib.sha256(source.read_bytes()).hexdigest()}
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"schema_version": 1, "start": 0,
                                            "rows": 3, "shards": [shard]}))
            with self.assertRaisesRegex(ValueError, "cover"):
                verify(manifest)
            shard["start"] = 1
            manifest.write_text(json.dumps({"schema_version": 1, "start": 0,
                                            "rows": 2, "shards": [shard]}))
            with self.assertRaisesRegex(ValueError, "gap"):
                verify(manifest)

    def test_rejects_symlinked_valid_shard(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            original = root / "actual.tsv"
            original.write_bytes(b"0\n1\n")
            link = root / "alias.tsv"
            link.symlink_to(original)
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"schema_version": 1, "start": 0,
                "rows": 2, "shards": [{"path": str(link), "start": 0,
                "rows": 2, "sha256": hashlib.sha256(original.read_bytes()).hexdigest()}]}))
            with self.assertRaisesRegex(ValueError, "symlink"):
                verify(manifest)

    def test_detects_extra_rows(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "rows.tsv"
            path.write_bytes(b"1\n2\n3\n")
            with self.assertRaisesRegex(ValueError, "additional"):
                verify_file(path, 1, 2)


if __name__ == "__main__":
    unittest.main()

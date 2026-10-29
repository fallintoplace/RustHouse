import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_shards import split_tsv


class ShardTests(unittest.TestCase):
    def test_shards_are_ordered_hashed_and_bounded(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "input.tsv"
            source.write_bytes(b"1\n2\n3\n4\n5\n")
            target = Path(root) / "shards"
            report = split_tsv(source, target, rows_per_chunk=2)
            self.assertEqual(report["total_rows"], 5)
            self.assertEqual([s["rows"] for s in report["shards"]], [2, 2, 1])
            self.assertEqual(b"".join((target / s["path"]).read_bytes()
                                       for s in report["shards"]), source.read_bytes())
            for shard in report["shards"]:
                self.assertEqual(hashlib.sha256((target / shard["path"]).read_bytes()).hexdigest(),
                                 shard["sha256"])
            self.assertTrue((target / "manifest.json").is_file())

    def test_large_shard_batches_preserve_hashes_across_flushes(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "big.tsv"
            records = [f"{i:08d},".encode() + b"x" * 1024 + b"\n"
                       for i in range(700)]
            source.write_bytes(b"".join(records))
            target = Path(root) / "shards"
            report = split_tsv(source, target, rows_per_chunk=350)
            self.assertEqual([part["rows"] for part in report["shards"]], [350, 350])
            for part in report["shards"]:
                content = (target / part["path"]).read_bytes()
                self.assertEqual(hashlib.sha256(content).hexdigest(), part["sha256"])
                self.assertEqual(len(content), part["bytes"])
            self.assertEqual(b"".join((target / p["path"]).read_bytes()
                                      for p in report["shards"]), source.read_bytes())

    def test_does_not_publish_malformed_source(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "input.tsv"
            source.write_bytes(b"ok\ntruncated")
            target = Path(root) / "shards"
            with self.assertRaisesRegex(ValueError, "truncated"):
                split_tsv(source, target)
            self.assertFalse(target.exists())

    @unittest.skipIf(sys.platform == "win32", "Windows runners cannot create symlinks")
    def test_refuses_to_replace_dangling_symlink(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "input.tsv"
            source.write_text("1\n")
            target = Path(root) / "shards"
            target.symlink_to(Path(root) / "missing", target_is_directory=True)
            with self.assertRaises(FileExistsError):
                split_tsv(source, target)
            self.assertTrue(target.is_symlink())

    def test_refuses_to_replace_existing_directory(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "input.tsv"
            source.write_text("1\n")
            target = Path(root) / "shards"
            target.mkdir()
            with self.assertRaises(FileExistsError):
                split_tsv(source, target)


if __name__ == "__main__":
    unittest.main()

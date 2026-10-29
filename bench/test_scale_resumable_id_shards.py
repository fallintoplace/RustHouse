import json
import sys
import tempfile
import unittest
from pathlib import Path

from bench.scale.resumable_id_shards import create


class ResumableShardsTests(unittest.TestCase):
    def test_creates_and_reuses_checked_shards(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            first = create(root, 12, 7, 3)
            self.assertEqual([entry["rows"] for entry in first["shards"]], [3, 3, 1])
            self.assertTrue(all(not item["reused"] for item in first["shards"]))
            self.assertEqual((root / "manifest.json").exists(), True)
            self.assertEqual(json.loads((root / "manifest.json").read_text())["rows"], 7)
            again = create(root, 12, 7, 3)
            self.assertTrue(all(item["reused"] for item in again["shards"]))
            self.assertEqual([x["sha256"] for x in first["shards"]],
                             [x["sha256"] for x in again["shards"]])

    def test_rejects_corruption_without_replacing_it(self):
        with tempfile.TemporaryDirectory() as temp:
            first = create(temp, 0, 5, 5)
            path = Path(first["shards"][0]["path"])
            path.write_bytes(b"0\n1\n3\n")
            with self.assertRaisesRegex(ValueError, "incomplete|invalid"):
                create(temp, 0, 5, 5)
            self.assertEqual(path.read_bytes(), b"0\n1\n3\n")

    @unittest.skipIf(sys.platform == "win32", "Windows CI cannot create symlinks")
    def test_refuses_existing_symlink_even_with_correct_rows(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            outside = root / "external.tsv"
            outside.write_bytes(b"0\n1\n")
            shard = root / "id-00000000000000000000-00000000000000000002.tsv"
            shard.symlink_to(outside)
            with self.assertRaisesRegex(ValueError, "symlink"):
                create(root, 0, 2, 2)

    def test_rejects_invalid_range(self):
        with tempfile.TemporaryDirectory() as temp:
            for start, rows, shard in [(0, 0, 10), (-1, 1, 1), (0, 1, 0),
                                       (2**64 - 1, 2, 1)]:
                with self.assertRaises(ValueError):
                    create(temp, start, rows, shard)

    @unittest.skipIf(sys.platform == "win32", "Windows CI cannot create symlinks")
    def test_rejects_symlinked_output_directory(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            outside = root / "outside"
            outside.mkdir()
            alias = root / "alias"
            alias.symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlinked shard directory"):
                create(alias, 0, 3, 2)
            self.assertEqual(list(outside.iterdir()), [])


if __name__ == "__main__":
    unittest.main()

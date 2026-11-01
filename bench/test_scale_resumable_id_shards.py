import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

from bench.scale.resumable_id_shards import create


class ResumableShardsTests(unittest.TestCase):
    def test_stale_pid_pending_files_are_not_removed(self):
        import os
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            folder.mkdir(exist_ok=True)
            shard = folder / "id-00000000000000000000-00000000000000000001.tsv"
            old_shard_pending = folder / f".{shard.name}.{os.getpid()}.pending"
            old_manifest_pending = folder / f".manifest.{os.getpid()}.pending"
            old_shard_pending.write_bytes(b"stale shard")
            old_manifest_pending.write_bytes(b"stale manifest")
            manifest = create(folder, 0, 1, 1)
            self.assertEqual(manifest["rows"], 1)
            self.assertEqual(old_shard_pending.read_bytes(), b"stale shard")
            self.assertEqual(old_manifest_pending.read_bytes(), b"stale manifest")

    def test_random_pending_collision_never_deletes_existing_file(self):
        from unittest.mock import patch
        import os
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            shard = folder / "id-00000000000000000000-00000000000000000001.tsv"
            pending = folder / f".{shard.name}.{os.getpid()}.collision.pending"
            pending.write_bytes(b"keep")
            with patch("bench.scale.resumable_id_shards.secrets.token_hex", return_value="collision"):
                with self.assertRaises(FileExistsError):
                    create(folder, 0, 1, 1)
            self.assertEqual(pending.read_bytes(), b"keep")
            self.assertFalse(shard.exists())

    @unittest.skipIf(sys.platform == "win32", "Windows CI cannot create symlinks")
    def test_direct_reuse_verification_refuses_symlinked_input(self):
        from bench.scale.resumable_id_shards import validate_existing
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            original = folder / "original.tsv"
            original.write_bytes(b"0\n1\n")
            alias = folder / "alias.tsv"
            alias.symlink_to(original)
            with self.assertRaisesRegex(ValueError, "regular file"):
                validate_existing(alias, 0, 2)
            self.assertEqual(
                validate_existing(original, 0, 2),
                hashlib.sha256(original.read_bytes()).hexdigest(),
            )

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

    def test_multi_batch_shard_retains_exact_rows_and_hash(self):
        import hashlib
        with tempfile.TemporaryDirectory() as temp:
            start, count = 2**64 - 8193, 8193
            manifest = create(temp, start, count, count)
            shard = Path(manifest["shards"][0]["path"])
            data = shard.read_bytes()
            self.assertEqual(data.splitlines()[0], str(start).encode("ascii"))
            self.assertEqual(data.splitlines()[-1], str(2**64 - 1).encode("ascii"))
            self.assertEqual(len(data.splitlines()), count)
            self.assertEqual(manifest["shards"][0]["sha256"],
                             hashlib.sha256(data).hexdigest())
            self.assertTrue(create(temp, start, count, count)["shards"][0]["reused"])

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


    def test_ctime_change_during_read_is_rejected(self):
        import os
        from types import SimpleNamespace
        from unittest.mock import patch

        with tempfile.TemporaryDirectory() as root:
            from bench.scale.resumable_id_shards import validate_existing
            path = Path(root) / "reused.tsv"
            path.write_bytes(b"0\n1\n")
            real_fstat = os.fstat
            calls = 0

            def changed_ctime(fd):
                nonlocal calls
                current = real_fstat(fd)
                calls += 1
                if calls != 2:
                    return current
                return SimpleNamespace(
                    st_mode=current.st_mode, st_dev=current.st_dev,
                    st_ino=current.st_ino, st_size=current.st_size,
                    st_mtime_ns=current.st_mtime_ns,
                    st_ctime_ns=current.st_ctime_ns + 1,
                )

            with patch("bench.scale.resumable_id_shards.os.fstat", side_effect=changed_ctime):
                with self.assertRaisesRegex(ValueError, "changed while reading"):
                    validate_existing(path, 0, 2)
            self.assertEqual(calls, 2)

    def test_reuse_verification_detects_corruption_across_batch_boundary(self):
        with tempfile.TemporaryDirectory() as root:
            first = create(root, 100, 8193, 8193)
            path = Path(first["shards"][0]["path"])
            before = path.read_bytes()
            reused = create(root, 100, 8193, 8193)
            self.assertTrue(reused["shards"][0]["reused"])
            self.assertEqual(reused["shards"][0]["sha256"], first["shards"][0]["sha256"])
            bad = before.replace(b"4196\n", b"4195\n", 1)
            path.write_bytes(bad)
            with self.assertRaisesRegex(ValueError, "invalid or incomplete"):
                create(root, 100, 8193, 8193)

if __name__ == "__main__":
    unittest.main()

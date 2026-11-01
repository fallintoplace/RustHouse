"""Do not partially load shuffled or skipped TSV shard manifests."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from bench.scale.chunk_copy import load_chunks


def prepare(root, order):
    manifest = []
    for index in range(2):
        name = f"part-{index:06d}.tsv"
        payload = f"{index}\n".encode("ascii")
        (root / name).write_bytes(payload)
        manifest.append({"path": name, "rows": 1, "bytes": len(payload),
                         "sha256": hashlib.sha256(payload).hexdigest()})
    selected = [manifest[i] for i in order]
    (root / "manifest.json").write_text(json.dumps({
        "format": "rusthouse-tsv-shards-v1", "header": False,
        "total_rows": len(selected), "total_bytes": sum(x["bytes"] for x in selected),
        "shards": selected,
    }), encoding="utf-8")


class CanonicalShardOrderTests(unittest.TestCase):
    def test_rejects_shuffled_manifests_before_first_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prepare(root, [1, 0])
            copies = []
            with self.assertRaisesRegex(ValueError, "canonical part order"):
                load_chunks("rusthouse", "db", "events", root,
                            execute=lambda *a, **k: copies.append(a))
            self.assertEqual(copies, [])

    def test_rejects_missing_first_part_even_with_matching_totals(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prepare(root, [1])
            with self.assertRaisesRegex(ValueError, "canonical part order"):
                load_chunks("rusthouse", "db", "events", root, dry_run=True)

    def test_canonical_manifest_passes_preflight(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prepare(root, [0, 1])
            report = load_chunks("rusthouse", "db", "events", root, dry_run=True)
            self.assertEqual(report["shards_planned"], 2)
            self.assertTrue(report["preflight_verified"])


if __name__ == "__main__":
    unittest.main()

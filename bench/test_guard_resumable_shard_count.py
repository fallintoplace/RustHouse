import tempfile
import unittest
from pathlib import Path

from bench.scale.resumable_id_shards import create


class ShardCountLimitTests(unittest.TestCase):
    def test_rejects_oversized_shard_plan_before_creating_directory(self):
        with tempfile.TemporaryDirectory() as temp:
            target = Path(temp) / "shards"
            with self.assertRaisesRegex(ValueError, "16384-part"):
                create(target, 0, 16385, 1)
            self.assertFalse(target.exists())

    def test_accepts_bounded_shard_plan(self):
        with tempfile.TemporaryDirectory() as temp:
            report = create(Path(temp) / "shards", 0, 4, 2)
            self.assertEqual(len(report["shards"]), 2)


if __name__ == "__main__":
    unittest.main()

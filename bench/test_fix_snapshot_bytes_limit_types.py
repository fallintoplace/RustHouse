import tempfile
import unittest
from bench.scale.snapshot_bytes import inventory

class SnapshotInventoryLimitTests(unittest.TestCase):
    def test_rejects_noninteger_limits(self):
        with tempfile.TemporaryDirectory() as root:
            for value in (True, 1.5, 0):
                with self.subTest(value=value), self.assertRaisesRegex(ValueError, "max_files"):
                    inventory(root, max_files=value)

    def test_accepts_valid_integer_limit(self):
        with tempfile.TemporaryDirectory() as root:
            report = inventory(root, max_files=1)
            self.assertEqual(report["file_count"], 0)

if __name__ == "__main__":
    unittest.main()

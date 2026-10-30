import tempfile
import unittest
from bench.scale.snapshot_fingerprint import fingerprint

class FingerprintLimitTests(unittest.TestCase):
    def test_rejects_noninteger_limits(self):
        with tempfile.TemporaryDirectory() as root:
            for kw in ({"max_files": True}, {"max_files": 1.5},
                       {"block_size": True}, {"block_size": 4096.5}):
                with self.subTest(kw=kw), self.assertRaisesRegex(ValueError, "invalid max_files"):
                    fingerprint(root, **kw)

if __name__ == "__main__":
    unittest.main()

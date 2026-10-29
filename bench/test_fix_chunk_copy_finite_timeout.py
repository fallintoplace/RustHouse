import math
import unittest
from bench.scale.chunk_copy import load_chunks

class CopyTimeoutTests(unittest.TestCase):
    def test_invalid_timeout_precedes_manifest_access(self):
        for timeout in (True, "1", 0, -1, math.nan, math.inf):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "timeout"):
                load_chunks("rusthouse", "db", "events", "/nonexistent-dir",
                            timeout=timeout, execute=lambda *args, **kwargs: self.fail("COPY started"))

if __name__ == "__main__":
    unittest.main()

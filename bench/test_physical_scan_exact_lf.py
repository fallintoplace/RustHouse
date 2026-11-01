"""Physical scan generated TSV must have LF on every platform."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.physical_scan import write_fixture


class ExactLfFixtureTests(unittest.TestCase):
    def test_uneven_batches_are_byte_exact(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "rows.tsv"
            write_fixture(path, 5, batch_rows=2)
            self.assertEqual(path.read_bytes(), b"0\n1\n2\n3\n4\n")


if __name__ == "__main__":
    unittest.main()

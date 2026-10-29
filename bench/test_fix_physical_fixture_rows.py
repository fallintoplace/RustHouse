import tempfile
import unittest
from pathlib import Path
from bench.scale.physical_scan import write_fixture

class PhysicalFixtureInputTests(unittest.TestCase):
    def test_invalid_rows_do_not_create_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "fixture.tsv"
            for rows in (0, True, 1.5, -1):
                with self.subTest(rows=rows), self.assertRaisesRegex(ValueError, "fixture rows"):
                    write_fixture(path, rows)
                self.assertFalse(path.exists())

    def test_valid_batches_keep_sequential_ids(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "fixture.tsv"
            write_fixture(path, 4, batch_rows=2)
            self.assertEqual(path.read_bytes(), b"0\n1\n2\n3\n")

if __name__ == "__main__":
    unittest.main()

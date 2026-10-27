import tempfile
import unittest
from pathlib import Path

from run import verify_result, write_fixture

class JoinWorkloadTests(unittest.TestCase):
    def test_small_fixture_and_matches(self):
        with tempfile.TemporaryDirectory() as directory:
            expected = write_fixture(Path(directory), 20, 5)
            self.assertEqual(expected, {0, 4, 8, 12, 16})
            facts = (Path(directory) / "facts.tsv").read_text().splitlines()
            dimensions = (Path(directory) / "dimensions.tsv").read_text().splitlines()
            self.assertEqual(len(facts), 21)
            self.assertEqual(len(dimensions), 6)
            verify_result(b"0\tgroup-0\n4\tgroup-1\n8\tgroup-2\n12\tgroup-3\n16\tgroup-4\n", expected)

    def test_invalid_or_duplicate_rows_are_rejected(self):
        expected = {0, 4}
        with self.assertRaises(ValueError):
            verify_result(b"0\tok\n0\tduplicate\n", expected)
        with self.assertRaises(ValueError):
            verify_result(b"0\tok\n4\n", expected)
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                write_fixture(directory, 20, 32769)

if __name__ == "__main__":
    unittest.main()

import tempfile
import unittest
from pathlib import Path

from run import verify_result, write_fixture

class JoinWorkloadTests(unittest.TestCase):
    def test_small_fixture_and_matches(self):
        with tempfile.TemporaryDirectory() as directory:
            expected = write_fixture(Path(directory), 20, 5)
            self.assertEqual(
                expected,
                {0: "group-0", 4: "group-1", 8: "group-2", 12: "group-3", 16: "group-4"},
            )
            facts = (Path(directory) / "facts.tsv").read_text().splitlines()
            dimensions = (Path(directory) / "dimensions.tsv").read_text().splitlines()
            self.assertEqual(len(facts), 21)
            self.assertEqual(len(dimensions), 6)
            verify_result(b"0\tgroup-0\n4\tgroup-1\n8\tgroup-2\n12\tgroup-3\n16\tgroup-4\n", expected)

    def test_requires_lf_terminated_join_rows(self):
        expected = {0: "group-0"}
        for bad in [b"0\tgroup-0", b"0\tgroup-0\r\n", b"0\tgroup-0\r"]:
            with self.assertRaises(ValueError):
                verify_result(bad, expected)
        verify_result(b"0\tgroup-0\n", expected)
        verify_result(b"", {})

    def test_invalid_or_duplicate_rows_are_rejected(self):
        expected = {0: "group-0", 4: "group-1"}
        with self.assertRaises(ValueError):
            verify_result(b"0\tgroup-0\n0\tduplicate\n", expected)
        with self.assertRaises(ValueError):
            verify_result(b"0\tgroup-0\n4\n", expected)
        with self.assertRaises(ValueError):
            verify_result(b"0\tgroup-1\n4\tgroup-1\n", expected)
        with self.assertRaises(ValueError):
            verify_result(b"0\tgroup-0\n8\tgroup-1\n", expected)
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                write_fixture(directory, 20, 32769)

if __name__ == "__main__":
    unittest.main()

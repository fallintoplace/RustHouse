import tempfile
import unittest
from pathlib import Path

from run import QUERY, logical_probe_throughput, main, query_for_strictness, verify_result, write_fixture

class JoinWorkloadTests(unittest.TestCase):
    def test_join_throughput_rejects_invalid_medians(self):
        self.assertEqual(logical_probe_throughput(20, 2.0), 10.0)
        for median in [0, -1.0, float("nan"), float("inf")]:
            with self.assertRaises(ValueError):
                logical_probe_throughput(20, median)

    def test_nonfinite_timeouts_are_rejected_before_launch(self):
        for value in ["nan", "inf", "-inf"]:
            with self.assertRaises(SystemExit) as raised:
                main(["--binary", "unused", f"--timeout={value}"])
            self.assertEqual(raised.exception.code, 2)

    def test_any_join_variant_uses_same_columns_and_predicate(self):
        self.assertEqual(query_for_strictness("all"), QUERY)
        self.assertIn("facts f ANY JOIN dimensions d", query_for_strictness("any"))
        with self.assertRaises(ValueError):
            query_for_strictness("full")

    def test_fixture_batch_boundaries_preserve_records(self):
        with tempfile.TemporaryDirectory() as directory:
            write_fixture(directory, 4097, 4096)
            lines = (Path(directory) / "facts.tsv").read_text().splitlines()
            self.assertEqual(len(lines), 4098)
            self.assertEqual(lines[4096], "4095\t95")
            self.assertEqual(lines[4097], "4096\t96")
            build = (Path(directory) / "dimensions.tsv").read_text().splitlines()
            self.assertEqual(len(build), 4097)
            self.assertEqual(build[-1], "4095\tgroup-5")

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

    def test_duplicate_dimension_keys_cover_all_and_any_join_shapes(self):
        with tempfile.TemporaryDirectory() as root:
            expected = write_fixture(root, 12, 6, duplicate_factor=2)
            self.assertEqual(sorted(expected), [0, 4, 8])
            rows = (Path(root) / "dimensions.tsv").read_text().splitlines()
            self.assertEqual(len(rows), 7)
            self.assertEqual(rows[1], rows[2])
            all_matches = (
                b"0\tgroup-0\n0\tgroup-0\n"
                b"4\tgroup-1\n4\tgroup-1\n"
                b"8\tgroup-2\n8\tgroup-2\n"
            )
            verify_result(all_matches, expected, multiplicity=2)
            any_matches = b"0\tgroup-0\n4\tgroup-1\n8\tgroup-2\n"
            verify_result(any_matches, expected, multiplicity=1)
            with self.assertRaises(ValueError):
                verify_result(any_matches, expected, multiplicity=2)
            with self.assertRaisesRegex(ValueError, "duplicate factor"):
                write_fixture(root, 12, 5, duplicate_factor=2)

    def test_requires_lf_terminated_join_rows(self):
        expected = {0: "group-0"}
        for bad in [b"0\tgroup-0", b"0\tgroup-0\r\n", b"0\tgroup-0\r"]:
            with self.assertRaises(ValueError):
                verify_result(bad, expected)
        verify_result(b"0\tgroup-0\n", expected)
        verify_result(b"", {})

    def test_disjoint_fixture_produces_no_join_matches(self):
        with tempfile.TemporaryDirectory() as directory:
            expected = write_fixture(directory, 20, 5, disjoint=True)
            self.assertEqual(expected, {})
            dimensions = (Path(directory) / "dimensions.tsv").read_text().splitlines()
            self.assertEqual(dimensions[1], "20\tgroup-0")
            verify_result(b"", expected)

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
        for bad in [b"+0\tgroup-0\n4\tgroup-1\n", b"00\tgroup-0\n4\tgroup-1\n"]:
            with self.assertRaises(ValueError):
                verify_result(bad, expected)
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                write_fixture(directory, 20, 32769)

if __name__ == "__main__":
    unittest.main()

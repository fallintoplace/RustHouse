import tempfile
import unittest
from pathlib import Path
from q12_shipmode import benchmark, oracle


class Q12ShipmodeTests(unittest.TestCase):
    def test_indexed_oracle_handles_reordered_extra_columns(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "lineitem.tsv"
            path.write_text("extra\tl_shipmode\tl_receiptdate\tcomment\n"
                            "x\tAIR\t1994-12-31\ty\n"
                            "z\tSHIP\t1994-01-01\tw\n")
            self.assertEqual(oracle(path), b"AIR\t1\nSHIP\t1\n")


    def test_groups_and_sorts_only_matching_dates(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "lineitem.tsv"
            path.write_text("l_receiptdate\tl_shipmode\n"
                            "1994-01-01\tSHIP\n1994-12-31\tAIR\n"
                            "1994-01-02\tAIR\n1995-01-01\tAIR\n")
            self.assertEqual(oracle(path), b"AIR\t2\nSHIP\t1\n")
            good = benchmark("fake", "db", path, execute=lambda sql: b"AIR\t2\nSHIP\t1\n")
            self.assertEqual(good["results"][0]["status"], "passed")
            bad = benchmark("fake", "db", path, execute=lambda sql: b"SHIP\t1\nAIR\t2\n")
            self.assertEqual(bad["results"][0]["status"], "mismatch")

    def test_no_matches_outputs_no_groups(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "lineitem.tsv"
            path.write_text("l_receiptdate\tl_shipmode\n1995-01-01\tAIR\n")
            self.assertEqual(oracle(path), b"")


class Q12BoundedInputTests(unittest.TestCase):
    def test_refuses_oversized_reference_lines(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "lineitem.tsv"
            path.write_text("l_receiptdate\\tl_shipmode\\n"
                            "1994-01-01\\t" + "A" * 100 + "\\n")
            with patch("q12_shipmode.MAX_REFERENCE_LINE_CHARS", 40):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    oracle(path)

    def test_refuses_symlinked_reference_before_reading(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "lineitem.tsv"
            source.write_text("l_receiptdate\\tl_shipmode\\n")
            alias = Path(root) / "alias.tsv"
            try:
                alias.symlink_to(source)
            except (OSError, NotImplementedError):
                self.skipTest("symlinks unavailable")
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                oracle(alias)


class Q12InputShapeTests(unittest.TestCase):
    def test_rejects_corrupt_reference_records(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "lineitem.tsv"
            for payload in ("l_receiptdate\n1994-01-01\n",
                            "l_receiptdate\tl_shipmode\n1994-01-01\n",
                            "l_receiptdate\tl_shipmode\n1994-01-01\tAIR\tEXTRA\n"):
                path.write_text(payload)
                with self.subTest(payload=payload):
                    with self.assertRaisesRegex(ValueError, "columns|data fields"):
                        oracle(path)

if __name__ == "__main__":
    unittest.main()

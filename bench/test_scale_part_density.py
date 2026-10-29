import tempfile
import unittest
from pathlib import Path

from bench.scale.part_density import part_density


class PartDensityTests(unittest.TestCase):
    def test_summarizes_count_files(self):
        with tempfile.TemporaryDirectory() as root:
            for name, rows in (("all_1_1_0", 3), ("all_2_2_0", 9)):
                part = Path(root) / name
                part.mkdir()
                (part / "count.txt").write_text(f"{rows}\n")
            result = part_density(root)
            self.assertEqual(result["part_count"], 2)
            self.assertEqual(result["rows"], 12)
            self.assertEqual(result["median_rows_per_part"], 6)
            self.assertEqual(result["parts_below_8192_rows"], 2)

    def test_rejects_broken_counts(self):
        with tempfile.TemporaryDirectory() as root:
            part = Path(root) / "all_1_1_0"
            part.mkdir()
            (part / "count.txt").write_text("-1\n")
            with self.assertRaisesRegex(ValueError, "invalid"):
                part_density(root)

    def test_rejects_noncanonical_counts_and_total_overflow(self):
        with tempfile.TemporaryDirectory() as root:
            part = Path(root) / "all_1_1_0"
            part.mkdir()
            for payload in ("١٢\n", "10\n\n", " 10\n"):
                (part / "count.txt").write_text(payload)
                with self.assertRaisesRegex(ValueError, "invalid"):
                    part_density(root)
            (part / "count.txt").write_text(f"{2**64 - 1}\n")
            second = Path(root) / "all_2_2_0"
            second.mkdir()
            (second / "count.txt").write_text("1\n")
            with self.assertRaisesRegex(ValueError, "UInt64"):
                part_density(root)

    def test_rejects_empty_snapshot(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaisesRegex(ValueError, "no wide parts"):
                part_density(root)


if __name__ == "__main__":
    unittest.main()

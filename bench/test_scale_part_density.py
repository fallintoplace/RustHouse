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

    def test_estimates_compaction_pressure_without_pretending_to_plan_merges(self):
        with tempfile.TemporaryDirectory() as root:
            for index, size in enumerate((10, 10, 10, 10)):
                part = Path(root) / f"all_{index}_{index}_0"
                part.mkdir()
                (part / "count.txt").write_text(f"{size}\n")
            result = part_density(root, target_part_rows=25)
            self.assertEqual(result["optimistic_parts_after_compaction"], 2)
            self.assertEqual(result["optimistic_reclaimable_parts"], 2)
            self.assertEqual(result["target_rows_per_part"], 25)
            with self.assertRaisesRegex(ValueError, "target_part_rows"):
                part_density(root, target_part_rows=True)

    def test_percentiles_and_small_part_pressure(self):
        with tempfile.TemporaryDirectory() as root:
            for index, size in enumerate((1, 100, 10000)):
                part = Path(root) / f"all_{index}_{index}_0"
                part.mkdir()
                (part / "count.txt").write_text(f"{size}\n")
            result = part_density(root, small_part_rows=1000)
            self.assertEqual(result["small_part_count"], 2)
            self.assertAlmostEqual(result["small_part_row_share"], 101 / 10101)
            self.assertEqual(result["p90_rows_per_part"], 10000)
            self.assertEqual(result["p99_rows_per_part"], 10000)
            with self.assertRaisesRegex(ValueError, "small_part_rows"):
                part_density(root, small_part_rows=0)

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
            for payload in ("١٢\n", "10\n\n", " 10\n", "01\n"):
                (part / "count.txt").write_bytes(payload.encode("utf-8"))
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

    def test_rejects_oversized_count_file(self):
        with tempfile.TemporaryDirectory() as root:
            part = Path(root) / "all_1_1_0"
            part.mkdir()
            (part / "count.txt").write_text("9" * 100000 + "\n")
            with self.assertRaisesRegex(ValueError, "oversized count"):
                part_density(root)


    def test_rejects_count_file_replaced_after_path_validation(self):
        import os
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            part = root / "all_1_1_0"
            part.mkdir()
            count = part / "count.txt"
            count.write_text("10\n")
            replacement = part / "replacement.txt"
            replacement.write_text("99\n")
            real_open = os.open

            def swap_before_open(path, flags, *args, **kwargs):
                if Path(path) == count:
                    count.unlink()
                    replacement.rename(count)
                return real_open(path, flags, *args, **kwargs)

            with patch("bench.scale.part_density.os.open", side_effect=swap_before_open):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    part_density(root)

    def test_ctime_change_during_read_is_rejected(self):
        import os
        from types import SimpleNamespace
        from unittest.mock import patch

        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "part"
            path.mkdir()
            (path / "count.txt").write_bytes(b"12\n")
            real_fstat = os.fstat
            calls = 0

            def changed_ctime(fd):
                nonlocal calls
                current = real_fstat(fd)
                calls += 1
                if calls != 2:
                    return current
                return SimpleNamespace(
                    st_mode=current.st_mode, st_dev=current.st_dev,
                    st_ino=current.st_ino, st_size=current.st_size,
                    st_mtime_ns=current.st_mtime_ns,
                    st_ctime_ns=current.st_ctime_ns + 1,
                )

            with patch("bench.scale.part_density.os.fstat", side_effect=changed_ctime):
                with self.assertRaisesRegex(ValueError, "changed during read"):
                    part_density(root)
            self.assertEqual(calls, 2)

    def test_reports_exact_fractions_above_float_precision(self):
        with tempfile.TemporaryDirectory() as root:
            values = (2**53 + 1, 2**53 + 2)
            for index, value in enumerate(values):
                part = Path(root) / f"all_{index}_{index}_0"
                part.mkdir()
                (part / "count.txt").write_text(f"{value}\n")
            report = part_density(root)
            self.assertEqual(report["median_rows_per_part_exact_numerator"], sum(values))
            self.assertEqual(report["median_rows_per_part_exact_denominator"], 2)
            self.assertEqual(report["mean_rows_per_part_exact_numerator"], sum(values))
            self.assertEqual(report["mean_rows_per_part_exact_denominator"], 2)

if __name__ == "__main__":
    unittest.main()

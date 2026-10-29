"""Small part density statistics do not need a second row list."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.part_density import part_density


class PartDensitySmallStatsTests(unittest.TestCase):
    def test_small_part_threshold_is_exclusive(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for index, rows in enumerate((1, 8, 8, 12)):
                part = root / f"part-{index}"
                part.mkdir()
                (part / "count.txt").write_text(f"{rows}\\n", encoding="ascii")
            report = part_density(root, small_part_rows=8, target_part_rows=10)
            self.assertEqual(report["part_count"], 4)
            self.assertEqual(report["small_part_count"], 1)
            self.assertEqual(report["small_part_row_share"], 1 / 29)
            self.assertEqual(report["optimistic_parts_after_compaction"], 3)

    def test_all_parts_are_small(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            part = root / "part"
            part.mkdir()
            (part / "count.txt").write_text("3\\n", encoding="ascii")
            report = part_density(root, small_part_rows=4)
            self.assertEqual(report["small_part_count"], 1)
            self.assertEqual(report["small_part_row_share"], 1.0)

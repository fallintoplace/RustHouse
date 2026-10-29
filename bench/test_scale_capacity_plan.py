import unittest

from bench.scale.capacity_plan import plan


class CapacityPlanTests(unittest.TestCase):
    def test_billion_rows_fit_at_65536_rows_per_part(self):
        result = plan(1_000_000_000)
        self.assertTrue(result["feasible_under_assumptions"])
        self.assertEqual(result["parts_required"], 15259)
        self.assertEqual(result["minimum_rows_per_part_to_fit"], 61036)

    def test_flags_excess_parts(self):
        result = plan(1_000_000_000, rows_per_part=4096)
        self.assertFalse(result["part_limit_fit"])
        self.assertFalse(result["feasible_under_assumptions"])

    def test_flags_insufficient_disk_budget(self):
        result = plan(100, bytes_per_row=20, disk_budget_bytes=1999)
        self.assertFalse(result["disk_budget_fit"])
        self.assertEqual(result["estimated_uncompressed_bytes"], 2000)

    def test_compressed_row_width_changes_peak_disk_budget(self):
        actual = plan(100, bytes_per_row=20, stored_bytes_per_row=5,
                      disk_budget_bytes=1000, disk_copies=2)
        self.assertEqual(actual["estimated_uncompressed_bytes"], 2000)
        self.assertEqual(actual["estimated_stored_bytes"], 500)
        self.assertEqual(actual["estimated_peak_disk_bytes"], 1000)
        self.assertTrue(actual["disk_budget_fit"])
        with self.assertRaisesRegex(ValueError, "stored_bytes_per_row"):
            plan(100, stored_bytes_per_row=0)
        with self.assertRaisesRegex(ValueError, "stored_bytes_per_row"):
            plan(100, stored_bytes_per_row=True)

    def test_rejects_negative_and_oversized_options(self):
        with self.assertRaisesRegex(ValueError, "rows"):
            plan(0)
        with self.assertRaisesRegex(ValueError, "max_parts"):
            plan(5, max_parts=16385)

    def test_peak_ingest_budget_includes_staging_data(self):
        insufficient = plan(100, bytes_per_row=20, disk_budget_bytes=3000)
        self.assertEqual(insufficient["estimated_uncompressed_bytes"], 2000)
        self.assertEqual(insufficient["estimated_peak_disk_bytes"], 4000)
        self.assertFalse(insufficient["disk_budget_fit"])
        enough = plan(100, bytes_per_row=20, disk_budget_bytes=3000,
                      disk_copies=1)
        self.assertTrue(enough["disk_budget_fit"])
        with self.assertRaisesRegex(ValueError, "disk_copies"):
            plan(100, disk_copies=0)


if __name__ == "__main__":
    unittest.main()

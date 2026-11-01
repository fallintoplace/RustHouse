"""Model immutable-part byte ceilings alongside row and part limits."""
import unittest

from bench.scale.capacity_plan import plan


class ByteBoundedPartPlanningTests(unittest.TestCase):
    def test_byte_limit_can_drive_parts_even_when_rows_fit(self):
        plain = plan(10_000, rows_per_part=1000, max_parts=12, stored_bytes_per_row=20)
        bounded = plan(10_000, rows_per_part=1000, max_parts=12,
                       stored_bytes_per_row=20, max_part_bytes=10_000)
        self.assertTrue(plain["part_limit_fit"])
        self.assertEqual(plain["parts_required"], 10)
        self.assertEqual(bounded["effective_rows_per_part"], 500)
        self.assertEqual(bounded["parts_required"], 20)
        self.assertFalse(bounded["part_limit_fit"])
        self.assertFalse(bounded["feasible_under_assumptions"])

    def test_part_byte_limit_rounds_down_to_complete_rows(self):
        report = plan(100, rows_per_part=100, stored_bytes_per_row=12,
                      max_part_bytes=59)
        self.assertEqual(report["effective_rows_per_part"], 4)
        self.assertEqual(report["parts_required"], 25)
        self.assertEqual(report["max_part_bytes"], 59)
        self.assertEqual(plan(100, rows_per_part=100,
                              max_part_bytes=10_000)["parts_required"], 1)

    def test_fails_if_single_row_exceeds_part_ceiling(self):
        with self.assertRaisesRegex(ValueError, "cannot hold one stored row"):
            plan(10, stored_bytes_per_row=1000, max_part_bytes=999)
        for bad in (True, 0, 1.5, -1):
            with self.subTest(bad=bad), self.assertRaisesRegex(
                ValueError, "max_part_bytes"
            ):
                plan(10, max_part_bytes=bad)


if __name__ == "__main__":
    unittest.main()

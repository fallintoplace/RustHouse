import unittest

from bench.scale.capacity_plan import plan


class CapacityTypeTests(unittest.TestCase):
    def test_rejects_fractional_dimensions(self):
        for args in (
            (1.5, {}),
            (10, {"rows_per_part": 2.5}),
            (10, {"max_parts": 3.5}),
            (10, {"bytes_per_row": 1.5}),
            (10, {"disk_budget_bytes": 10.5}),
        ):
            with self.subTest(args=args), self.assertRaises(ValueError):
                plan(args[0], **args[1])

    def test_rejects_boolean_dimensions(self):
        for args in (
            (True, {}),
            (10, {"rows_per_part": True}),
            (10, {"max_parts": True}),
            (10, {"bytes_per_row": True}),
            (10, {"disk_budget_bytes": True}),
        ):
            with self.subTest(args=args), self.assertRaises(ValueError):
                plan(args[0], **args[1])

    def test_preserves_integer_plans(self):
        self.assertEqual(plan(100, rows_per_part=10)["parts_required"], 10)


if __name__ == "__main__":
    unittest.main()

"""Physical report comparison validates inputs before inspecting metadata."""
import unittest

from bench.scale.compare_physical import compare


class PhysicalComparisonShapeTests(unittest.TestCase):
    def test_requires_two_report_objects(self):
        for value in (None, [], "not-json", 1):
            with self.subTest(value=value):
                with self.assertRaisesRegex(ValueError, "objects"):
                    compare(value, {})
                with self.assertRaisesRegex(ValueError, "objects"):
                    compare({}, value)

    def test_object_reports_keep_existing_workload_validation(self):
        with self.assertRaisesRegex(ValueError, "persisted"):
            compare({}, {})

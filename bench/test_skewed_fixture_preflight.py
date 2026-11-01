"""Ensure invalid skewed fixture requests have no filesystem side effects."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.skewed_fixture import generate


class SkewedFixturePreflightTests(unittest.TestCase):
    def test_invalid_controls_do_not_create_output_parent(self):
        with tempfile.TemporaryDirectory() as root:
            for overrides in ({"rows": 0}, {"groups": 0},
                              {"hot_percent": 101}, {"batch_rows": 0}):
                with self.subTest(overrides=overrides):
                    parent = Path(root) / "new"
                    options = {"rows": 101, "groups": 4, "hot_percent": 90,
                               "batch_rows": 7}
                    options.update(overrides)
                    with self.assertRaises(ValueError):
                        generate(parent / "rows.tsv", **options)
                    self.assertFalse(parent.exists())

    def test_valid_output_is_unchanged(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "rows.tsv"
            generate(path, 101, groups=4, hot_percent=90, batch_rows=7)
            lines = path.read_text(encoding="ascii").splitlines()
            self.assertEqual(len(lines), 101)
            self.assertEqual(lines[0], "0\t0")
            self.assertEqual(lines[90], "90\t1")
            self.assertEqual(lines[100], "100\t0")


if __name__ == "__main__":
    unittest.main()

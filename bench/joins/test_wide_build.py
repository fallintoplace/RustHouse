"""Verify wide JOIN fixtures without a RustHouse installation."""
import tempfile
import unittest
from pathlib import Path

from bench.joins.wide_build import write_dimensions


class WideJoinFixtureTests(unittest.TestCase):
    def test_identical_join_keys_with_unreferenced_wide_column(self):
        with tempfile.TemporaryDirectory() as temp:
            narrow, wide = write_dimensions(temp, 27, 1536)
            small = narrow.read_text().splitlines()
            large = wide.read_text().splitlines()
            self.assertEqual(small[0], "id")
            self.assertEqual(large[0], "id\tpayload")
            self.assertEqual(len(small), 28)
            self.assertEqual(len(large), 28)
            self.assertEqual([int(x) for x in small[1:]],
                             [int(x.split("\t")[0]) for x in large[1:]])
            self.assertTrue(all(len(x.split("\t")[1]) == 1536 for x in large[1:]))

    def test_limits_prevent_unbounded_fixture_disk_use(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(ValueError):
                write_dimensions(temp, 32768, 65536)
            with self.assertRaises(ValueError):
                write_dimensions(temp, 0, 1)
            with self.assertRaises(ValueError):
                write_dimensions(temp, 10, 0)


if __name__ == "__main__":
    unittest.main()

"""Fixture validation should not create directories for invalid requests."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.permuted_ids import generate


class PermutedInputPreflightTests(unittest.TestCase):
    def test_invalid_controls_leave_parent_uncreated(self):
        with tempfile.TemporaryDirectory() as directory:
            for overrides in (
                {"rows": 0}, {"rows": True},
                {"seed": -1}, {"seed": True}, {"seed": 2 ** 64},
                {"batch_rows": 0}, {"batch_rows": 1_000_001},
            ):
                with self.subTest(overrides=overrides):
                    target = Path(directory) / "not-created" / "ids.tsv"
                    arguments = {"rows": 4, "seed": 1, "batch_rows": 2}
                    arguments.update(overrides)
                    with self.assertRaises(ValueError):
                        generate(target, **arguments)
                    self.assertFalse(target.parent.exists())

    def test_valid_permutation_is_still_generated(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "created" / "ids.tsv"
            generate(target, 4, seed=1, batch_rows=2)
            lines = [int(value) for value in target.read_text().splitlines()]
            self.assertEqual(len(lines), 4)
            self.assertEqual(len(set(lines)), 4)


if __name__ == "__main__":
    unittest.main()

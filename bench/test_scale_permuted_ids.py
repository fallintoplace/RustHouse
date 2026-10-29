import io
import tempfile
import unittest
from pathlib import Path

from bench.scale.permuted_ids import MASK, emit, generate, permuted


class PermutedFixturesTests(unittest.TestCase):
    def test_unique_first_4096_ids_without_a_shuffle_buffer(self):
        values = [permuted(index, 19) for index in range(4096)]
        self.assertEqual(len(set(values)), len(values))
        self.assertTrue(all(0 <= value <= MASK for value in values))

    def test_is_repeatable_across_batch_sizes(self):
        first, second = io.StringIO(), io.StringIO()
        emit(first, 19, seed=42, batch_rows=3)
        emit(second, 19, seed=42, batch_rows=10)
        self.assertEqual(first.getvalue(), second.getvalue())

    def test_inlined_permutation_matches_checked_function(self):
        output = io.StringIO()
        rows, seed = 10017, MASK - 9
        emit(output, rows, seed=seed, batch_rows=137)
        expected = "".join(f"{permuted(i, seed)}\n" for i in range(rows))
        self.assertEqual(output.getvalue(), expected)

    def test_rejects_domain_overflow(self):
        with self.assertRaisesRegex(ValueError, "UInt64"):
            permuted(MASK + 1)
        with self.assertRaisesRegex(ValueError, "UInt64"):
            emit(io.StringIO(), 1, seed=-1)

    def test_writes_new_file(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "ids.tsv"
            generate(path, 3, seed=5)
            self.assertEqual(path.read_text().splitlines(), [str(permuted(i, 5)) for i in range(3)])
            with self.assertRaises(FileExistsError):
                generate(path, 3)

    def test_never_replaces_raced_destination(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "ids.tsv"
            with patch("bench.scale.permuted_ids.os.link", side_effect=FileExistsError):
                with self.assertRaises(FileExistsError):
                    generate(path, 3)
            self.assertFalse(path.exists())
            self.assertEqual(list(Path(root).iterdir()), [])


    def test_modular_recurrence_wraps_and_preserves_exact_ids(self):
        for seed in (0, MASK, MASK - MULTIPLIER + 1):
            output = io.StringIO()
            emit(output, 11, seed=seed, batch_rows=2)
            self.assertEqual(output.getvalue(),
                             "".join(f"{permuted(i, seed)}\n" for i in range(11)))


if __name__ == "__main__":
    unittest.main()

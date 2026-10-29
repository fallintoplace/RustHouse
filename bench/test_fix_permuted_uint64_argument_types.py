import io
import unittest

from bench.scale.permuted_ids import MASK, emit, permuted


class PermutedUint64InputTests(unittest.TestCase):
    def test_rejects_noninteger_permutation_inputs(self):
        for index, seed in ((True, 0), (0.5, 0), (0, False), (0, 1.5)):
            with self.subTest(index=index, seed=seed), self.assertRaises(ValueError):
                permuted(index, seed)

    def test_rejects_invalid_stream_dimensions_before_writing(self):
        for rows, seed, batch_rows in ((True, 0, 4), (2.5, 0, 4),
                                       (2, True, 4), (2, 0, 1.5),
                                       (2, 0, True)):
            stream = io.StringIO()
            with self.subTest(rows=rows, seed=seed, batch_rows=batch_rows):
                with self.assertRaises(ValueError):
                    emit(stream, rows, seed=seed, batch_rows=batch_rows)
                self.assertEqual(stream.getvalue(), "")

    def test_preserves_integer_permutation(self):
        self.assertEqual(permuted(0, MASK), MASK)
        stream = io.StringIO()
        emit(stream, 3, seed=4, batch_rows=2)
        self.assertEqual(stream.getvalue().splitlines(),
                         [str(permuted(i, 4)) for i in range(3)])


if __name__ == "__main__":
    unittest.main()

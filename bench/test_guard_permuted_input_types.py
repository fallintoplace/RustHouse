import io
import unittest

from bench.scale.permuted_ids import emit, permuted


class PermutationIntegerInputsTests(unittest.TestCase):
    def test_rejects_noninteger_index_or_seed(self):
        for index, seed in ((True, 0), (1.5, 0), (0, False), (0, 0.5)):
            with self.subTest(index=index, seed=seed), self.assertRaises(ValueError):
                permuted(index, seed)

    def test_rejects_noninteger_rows_seed_and_batch_size(self):
        for kwargs in (
            {"rows": True}, {"rows": 1.5},
            {"rows": 1, "seed": False}, {"rows": 1, "seed": 1.5},
            {"rows": 1, "batch_rows": True}, {"rows": 1, "batch_rows": 2.5},
        ):
            rows = kwargs["rows"]
            options = {k: v for k, v in kwargs.items() if k != "rows"}
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                emit(io.StringIO(), rows, **options)


if __name__ == "__main__":
    unittest.main()

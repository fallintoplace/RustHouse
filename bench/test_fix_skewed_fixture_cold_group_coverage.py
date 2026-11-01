"""Regression coverage for cold-key cardinality under periodic hot skew."""
import io
import unittest

from bench.scale.skewed_fixture import emit


class ColdGroupCoverageTests(unittest.TestCase):
    def test_all_cold_groups_are_reachable_independent_of_batch_size(self):
        outputs = []
        for batch_rows in (1, 7, 8192):
            output = io.StringIO()
            emit(output, 10_000, groups=1000, hot_percent=90, batch_rows=batch_rows)
            outputs.append(output.getvalue())
        self.assertEqual(outputs[0], outputs[1])
        self.assertEqual(outputs[1], outputs[2])
        keys = [int(line.split("\t")[1]) for line in outputs[0].splitlines()]
        self.assertEqual(keys.count(0), 9000)
        self.assertEqual(set(keys) - {0}, set(range(1, 1001)))

    def test_all_hot_and_all_cold_boundaries(self):
        output = io.StringIO()
        emit(output, 8, groups=3, hot_percent=0, batch_rows=2)
        self.assertEqual([int(row.split("\t")[1]) for row in output.getvalue().splitlines()],
                         [1, 2, 3, 1, 2, 3, 1, 2])
        output = io.StringIO()
        emit(output, 8, groups=3, hot_percent=100, batch_rows=2)
        self.assertEqual({row.split("\t")[1] for row in output.getvalue().splitlines()}, {"0"})


if __name__ == "__main__":
    unittest.main()

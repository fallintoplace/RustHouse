"""Independent golden-oracle checks for large physical GROUP BY acceptance."""
import io
import unittest

from bench.scale.grouped_physical import emit, expected_tsv, reference, region_at


class GroupedPhysicalReferenceTests(unittest.TestCase):
    def test_matches_brute_force_for_skewed_and_boundary_inputs(self):
        for groups in (2, 3, 98, 4096):
            for rows in (1, 9, 10, 11, 97, 98, 999, 8192, 12345):
                expected_counts = [0] * groups
                expected_sums = [0] * groups
                for row in range(rows):
                    key = region_at(row, groups)
                    expected_counts[key] += 1
                    expected_sums[key] += 1 + row % 97
                counts, sums = reference(rows, groups)
                self.assertEqual(counts, expected_counts, (rows, groups))
                self.assertEqual(sums, expected_sums, (rows, groups))

    def test_real_billion_row_closed_form_oracle(self):
        rows = 1_000_000_000
        counts, sums = reference(rows, 4096)
        self.assertEqual(sum(counts), rows)
        cycles, tail = divmod(rows, 97)
        self.assertEqual(sum(sums), cycles * 4753 + tail * (tail + 1) // 2)
        self.assertEqual(counts[0], 900_000_000)
        self.assertEqual(sum(count > 0 for count in counts), 4096)
        self.assertEqual(expected_tsv(counts).splitlines()[0], b"0\t900000000")

    def test_emitted_csv_and_oracle_agree_for_uneven_prefix(self):
        sink = io.StringIO()
        emit(sink, 1111, 101, batch_rows=7)
        lines = sink.getvalue().splitlines()
        self.assertEqual(len(lines), 1111)
        counts, sums = reference(1111, 101)
        actual_counts = [0] * 101
        actual_sums = [0] * 101
        for line in lines:
            group, value = [int(item) for item in line.split("\t")]
            actual_counts[group] += 1
            actual_sums[group] += value
        self.assertEqual(counts, actual_counts)
        self.assertEqual(sums, actual_sums)


if __name__ == "__main__":
    unittest.main()

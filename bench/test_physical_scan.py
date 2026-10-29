import unittest

from bench.scale.physical_scan import benchmark, even_reference, p90


class PhysicalScanReferenceTests(unittest.TestCase):
    def test_even_count_and_sum_cover_odd_and_even_sizes(self):
        self.assertEqual(even_reference(1), (1, 0))
        self.assertEqual(even_reference(2), (1, 0))
        self.assertEqual(even_reference(3), (2, 2))
        self.assertEqual(even_reference(4), (2, 2))
        self.assertEqual(even_reference(5), (3, 6))
        self.assertEqual(
            even_reference(1_000_000_000),
            (500_000_000, 249_999_999_500_000_000),
        )

    def test_rejects_nonpositive_row_counts(self):
        for rows in (0, -1):
            with self.assertRaises(ValueError):
                even_reference(rows)

    def test_rejects_invalid_direct_benchmark_parameters(self):
        for rows, runs, warmups, timeout in (
            (True, 1, 0, 1), (1.0, 1, 0, 1), (1_000_000_001, 1, 0, 1),
            (100, 0, 0, 1), (100, 1.5, 0, 1), (100, 1, -1, 1),
            (100, 1, 0, float("nan")), (100, 1, 0, float("inf")),
        ):
            with self.subTest(rows=rows, runs=runs, warmups=warmups, timeout=timeout):
                with self.assertRaises(ValueError):
                    benchmark("unused", rows, runs, warmups, timeout)

    def test_nearest_rank_p90(self):
        self.assertEqual(p90([4.0, 1.0, 2.0, 3.0, 5.0]), 5.0)
        self.assertEqual(p90([10.0]), 10.0)
        with self.assertRaises(ValueError):
            p90([])

    def test_physical_fixture_bounded_batch_tuning(self):
        from bench.scale.physical_scan import write_fixture
        from pathlib import Path
        from tempfile import TemporaryDirectory
        with TemporaryDirectory() as root:
            fixture = Path(root) / "events.tsv"
            write_fixture(fixture, 5, batch_rows=2)
            self.assertEqual(fixture.read_bytes(), b"0\n1\n2\n3\n4\n")
            with self.assertRaisesRegex(ValueError, "batch_rows"):
                write_fixture(fixture, 5, batch_rows=1_000_000_001)


if __name__ == "__main__":
    unittest.main()

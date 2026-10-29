"""Offline fixture and oracle coverage for UInt32 DISTINCT RSS benchmark."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.distinct32_memory import distinct_values, write_tsv


class ExactDistinctMemoryFixtureTests(unittest.TestCase):
    def test_sparse_values_hit_different_high_sixteen_bit_ranges(self):
        values = list(distinct_values(32768, "sparse"))
        self.assertEqual(len(values), len(set(values)))
        self.assertEqual(values[0], 0)
        self.assertEqual(values[-1], 32767 << 16)
        self.assertTrue(all(0 <= v <= 0xffffffff for v in values))

    def test_dense_values_preserve_16bit_collisions(self):
        values = list(distinct_values(70000, "dense"))
        self.assertEqual(len(set(values)), 65536)
        self.assertEqual(values[0], values[65536])

    def test_mixed_input_remains_in_unsigned_32bit_domain(self):
        values = list(distinct_values(10000, "mixed"))
        self.assertTrue(all(0 <= v <= 0xffffffff for v in values))
        self.assertLessEqual(len(set(values)), len(values))

    def test_fixture_count_is_from_python_exact_oracle(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "events.tsv"
            unique = write_tsv(path, 73, "mixed", 3)
            lines = path.read_text().splitlines()
            self.assertEqual(lines[0], "id")
            self.assertEqual(len(lines), 1 + 73 * 3)
            self.assertEqual(unique, len({int(x) for x in lines[1:]}))

    def test_rejects_unbounded_input(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "events.tsv"
            with self.assertRaises(ValueError):
                write_tsv(path, 200000, "sparse", 16)
            with self.assertRaises(ValueError):
                write_tsv(path, 10, "sparse", 0)
        with self.assertRaises(ValueError):
            list(distinct_values(0, "sparse"))
        with self.assertRaises(ValueError):
            list(distinct_values(5, "bad"))

    def test_invalid_memory_benchmark_budgets_fail_before_loading_fixture(self):
        from bench.scale.distinct32_memory import benchmark
        for kwargs in (
            {"runs": True}, {"runs": 101}, {"runs": 0},
            {"timeout": float("nan")}, {"timeout": float("inf")},
            {"sample_ms": True}, {"sample_ms": 0},
            {"max_rss_mib": -1}, {"max_rss_mib": float("inf")},
        ):
            with self.subTest(kwargs=kwargs):
                with self.assertRaises(ValueError):
                    benchmark("no-binary", **kwargs)

    def test_buffer_flush_boundaries_preserve_all_duplicates(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "distinct.tsv"
            unique = write_tsv(source, 8193, "sparse", 2)
            lines = source.read_text().splitlines()
            self.assertEqual(unique, 8193)
            self.assertEqual(len(lines), 1 + 8193 * 2)
            self.assertEqual(lines[-1], lines[-2])
            self.assertEqual(lines[1], lines[2])


if __name__ == "__main__":
    unittest.main()

"""Boundary-aware sparse probes cover the first and last UInt64 IDs."""
import unittest

from bench.scale.sparse_point_probes import choose_ids, probe


class BoundaryAwareSamplingTests(unittest.TestCase):
    def test_extremes_and_uniform_unique_interiors(self):
        start = (1 << 64) - 1000
        values = choose_ids(start, 1000, 999, 17, include_boundaries=True)
        self.assertEqual(values[:2], [start, (1 << 64) - 1])
        self.assertEqual(len(values), len(set(values)))
        self.assertEqual(values, choose_ids(start, 1000, 999, 17,
                                           include_boundaries=True))
        self.assertTrue(all(start <= value < (1 << 64) for value in values))

    def test_small_domains_and_count_one(self):
        self.assertEqual(choose_ids(42, 1, 1, 5, include_boundaries=True), [42])
        self.assertEqual(choose_ids(42, 2, 2, 5, include_boundaries=True), [42, 43])
        self.assertEqual(choose_ids(42, 2, 1, 5, include_boundaries=True), [42])
        self.assertEqual(choose_ids(42, 3, 3, 5, include_boundaries=True), [42, 44, 43])
        with self.assertRaisesRegex(ValueError, "include_boundaries"):
            choose_ids(0, 2, 1, 1, include_boundaries=1)

    def test_probe_passes_boundary_ids_to_sql(self):
        observed = []
        def run(sql):
            value = int(sql.rsplit(" = ", 1)[1].split()[0])
            observed.append(value)
            return f"{value}\n".encode()
        result = probe("unused", "unused", "t", 70, 100, count=3,
                       seed=7, execute=run, include_boundaries=True)
        self.assertTrue(result["boundaries_included"])
        self.assertEqual(observed[:2], [70, 169])


if __name__ == "__main__":
    unittest.main()

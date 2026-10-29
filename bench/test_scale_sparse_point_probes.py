import unittest

from bench.scale.sparse_point_probes import choose_ids, percentile, probe


class SparseProbeTests(unittest.TestCase):
    def test_samples_unique_ids_in_a_billion_without_materializing(self):
        ids = choose_ids(0, 1_000_000_000, 100, 1234)
        self.assertEqual(len(ids), len(set(ids)))
        self.assertTrue(all(0 <= value < 1_000_000_000 for value in ids))
        self.assertEqual(ids, choose_ids(0, 1_000_000_000, 100, 1234))

    def test_verifies_queries_and_detects_missing_id(self):
        calls = []
        def run(statement):
            calls.append(statement)
            value = int(statement.rsplit(" = ", 1)[1].split()[0])
            return f"{value}\n".encode()
        report = probe("unused", "unused", "events", 8, 3, 3, execute=run)
        self.assertTrue(report["verified"])
        self.assertEqual(len(calls), 3)
        with self.assertRaisesRegex(ValueError, "point probe"):
            probe("unused", "unused", "events", 8, 3, 1,
                  execute=lambda _: b"")

    def test_nearest_rank_and_bad_bounds(self):
        self.assertEqual(percentile([1, 2, 3, 4, 5], 95), 5)
        with self.assertRaises(ValueError):
            choose_ids(0, 2, 3, 0)
        with self.assertRaises(ValueError):
            probe("x", "x", "a;drop", 0, 10)

    def test_sampling_supports_full_uint64_domain(self):
        first = choose_ids(0, 1 << 64, 10, 42)
        self.assertEqual(len(set(first)), 10)
        self.assertEqual(first, choose_ids(0, 1 << 64, 10, 42))
        self.assertTrue(all(0 <= value < (1 << 64) for value in first))


if __name__ == "__main__":
    unittest.main()

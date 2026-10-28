"""Deterministic paired timing tests without any live databases."""
import itertools
import unittest

from bench.analytics.clickhouse_parity import paired_query


class PairedQueryTests(unittest.TestCase):
    def test_alternates_call_order_and_preserves_medians(self):
        order = []
        def ch(query):
            order.append("ch")
            self.assertEqual(query, "SELECT count(*) FROM t FORMAT TabSeparated")
            return b"42\n"
        def rh(query):
            order.append("rh")
            self.assertEqual(query, "SELECT count(*) FROM t")
            return b"42\n"
        clock = itertools.count(0, 1_000_000).__next__
        report = paired_query("SELECT count(*) FROM t", rh, ch,
                              runs=2, warmups=1, clock=clock)
        self.assertEqual(order, ["rh", "ch", "ch", "rh", "rh", "ch"])
        self.assertEqual(report["rows"], 1)
        self.assertEqual(report["rusthouse_ms"], 1.0)
        self.assertEqual(report["clickhouse_ms"], 1.0)
        self.assertEqual(report["clickhouse_over_rusthouse"], 1.0)
        self.assertEqual(len(report["rusthouse_samples_ms"]), 2)

    def test_compares_all_warmup_and_measured_outputs(self):
        outputs = iter([b"2\n", b"2\n", b"3\n", b"2\n"])
        def rh(query):
            return next(outputs)
        def ch(query):
            return next(outputs)
        with self.assertRaises(ValueError):
            paired_query("SELECT count(*) FROM t", rh, ch,
                         runs=2, warmups=0, clock=itertools.count(0, 1_000_000).__next__)

    def test_rejects_incorrect_sample_counts(self):
        with self.assertRaises(ValueError):
            paired_query("SELECT 1", lambda _: b"1\n", lambda _: b"1\n", runs=0)
        with self.assertRaises(ValueError):
            paired_query("SELECT 1", lambda _: b"1\n", lambda _: b"1\n", warmups=-1)


if __name__ == "__main__":
    unittest.main()

import unittest

from bench.tpch.paired_clickhouse import benchmark_pair


class StrictPairedBenchmarkInputsTests(unittest.TestCase):
    def test_rejects_boolean_float_and_excessive_round_counts_before_execution(self):
        def unreachable(*args):
            self.fail("invalid benchmark must not run")
        for runs, warmups in [(True, 0), (1, True), (1.5, 0),
                              (0, 0), (1, -1), (251, 0), (1, 500)]:
            with self.subTest(runs=runs, warmups=warmups):
                with self.assertRaisesRegex(ValueError, "bounded integers"):
                    benchmark_pair("SELECT 1", b"1\n", unreachable,
                                   runs=runs, warmups=warmups)

    def test_rejects_bad_sample_type_and_overflow(self):
        for seconds in [True, "0.01", 10 ** 400, float("nan"), float("inf"), 0]:
            with self.subTest(seconds=str(seconds)[:30]):
                with self.assertRaisesRegex(ValueError, "timings"):
                    benchmark_pair("SELECT 1", b"1\n",
                                   lambda *_: (b"1\n", seconds), runs=1, warmups=0)

    def test_two_valid_rounds_preserve_four_samples_per_engine(self):
        result = benchmark_pair("SELECT 1", b"1\n",
                                lambda *_: (b"1\n", 0.5), runs=2, warmups=0)
        self.assertEqual(len(result["rusthouse_samples_seconds"]), 4)
        self.assertEqual(len(result["clickhouse_samples_seconds"]), 4)


if __name__ == "__main__":
    unittest.main()

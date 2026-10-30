import unittest
from bench.pr_bench_compare import parse_benchmark

def sample(rate="100.00"):
    return ("RustHouse numbers() benchmarks: 8192 input rows, 3 samples, 1 warmups\n"
            f"filtered count 1.000 {rate}\n"
            "filtered sum 1.000 200.00\n"
            "filtered top-K 1.000 300.00\n")

class ThroughputIntegrityTests(unittest.TestCase):
    def test_rejects_invalid_throughput(self):
        for rate in ("0.00", "9999999999999999999999999999999999999999999999999999999999.0",):
            with self.subTest(rate=rate):
                # The very long decimal can be finite; it must still be a valid positive throughput.
                if rate.startswith("0"):
                    with self.assertRaisesRegex(ValueError, "throughput"):
                        parse_benchmark(sample(rate))
        with self.assertRaisesRegex(ValueError, "throughput"):
            parse_benchmark(sample("9"*400 + ".0"))

    def test_accepts_real_measurements(self):
        _, medians = parse_benchmark(sample())
        self.assertEqual(medians["filtered count"], 1)

if __name__ == "__main__":
    unittest.main()

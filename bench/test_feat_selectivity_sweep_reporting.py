import unittest
from types import SimpleNamespace
from bench.scale.selectivity_sweep import run_sweep

class SelectivityMetricsTests(unittest.TestCase):
    def test_exact_selectivity_and_tail_latency(self):
        def execute(args, **kwargs):
            boundary = int(args[2].split(" >= ")[1])
            return SimpleNamespace(returncode=0,
                                   stdout=f"{4-boundary}\n".encode("ascii"), stderr=b"")
        report = run_sweep("rusthouse", ".", 4, steps=2, samples=1, execute=execute)
        self.assertEqual([r["selectivity_percent"] for r in report["results"]],
                         [100.0, 50.0, 0.0])
        for item in report["results"]:
            self.assertTrue(item["verified"])
            self.assertEqual(item["p95_seconds"], item["samples_seconds"][0])

if __name__ == "__main__":
    unittest.main()

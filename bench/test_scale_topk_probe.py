import unittest
from types import SimpleNamespace

from bench.scale.topk_probe import probe


class TopKProbeTests(unittest.TestCase):
    def test_verifies_both_orders(self):
        def execute(command, **kwargs):
            desc = "DESC" in command[2]
            output = b"4\n3\n" if desc else b"0\n1\n"
            return SimpleNamespace(returncode=0, stdout=output, stderr=b"")
        result = probe("rusthouse", "parts", 5, limit=2, execute=execute)
        self.assertEqual([r["direction"] for r in result["results"]], ["ASC", "DESC"])
        self.assertTrue(all(r["verified"] for r in result["results"]))

    def test_rejects_bad_identifiers_and_limits(self):
        with self.assertRaisesRegex(ValueError, "identifier"):
            probe("rusthouse", "part", 5, column="id;DROP", limit=2)
        with self.assertRaisesRegex(ValueError, "limit"):
            probe("rusthouse", "part", 5, limit=6)

    def test_detects_wrong_order(self):
        with self.assertRaisesRegex(ValueError, "differs"):
            probe("rusthouse", "part", 3, limit=1,
                  execute=lambda *a, **k: SimpleNamespace(returncode=0, stdout=b"9\n", stderr=b""))


if __name__ == "__main__":
    unittest.main()

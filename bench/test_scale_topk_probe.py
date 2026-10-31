import unittest
from unittest.mock import patch
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

    def test_default_topk_matches_short_fixture_and_keeps_explicit_validation(self):
        def execute(command, **kwargs):
            self.assertIn("LIMIT 3", command[2])
            values = b"2\n1\n0\n" if "DESC" in command[2] else b"0\n1\n2\n"
            return SimpleNamespace(returncode=0, stdout=values, stderr=b"")

        result = probe("rusthouse", "part", 3, execute=execute)
        self.assertEqual([case["limit"] for case in result["results"]], [3, 3])
        self.assertTrue(all(case["verified"] for case in result["results"]))
        with self.assertRaisesRegex(ValueError, "limit"):
            probe("rusthouse", "part", 3, limit=4, execute=execute)

    def test_nonzero_start_uses_exact_uint64_endpoints(self):
        boundary = (1 << 64) - 3
        expected = [f"{boundary}\n{boundary + 1}\n".encode(),
                    f"{boundary + 2}\n{boundary + 1}\n".encode()]
        calls = []
        def run(command, **kwargs):
            calls.append(command[2])
            return SimpleNamespace(returncode=0, stdout=expected[len(calls)-1], stderr=b"")
        result = probe("rusthouse", "part", 3, start=boundary, limit=2, execute=run)
        self.assertEqual(result["start"], boundary)
        self.assertEqual(len(calls), 2)
        with self.assertRaisesRegex(ValueError, "overflow"):
            probe("rusthouse", "part", 4, start=boundary)

    def test_rejects_bad_identifiers_and_limits(self):
        with self.assertRaisesRegex(ValueError, "identifier"):
            probe("rusthouse", "part", 5, column="id;DROP", limit=2)
        with self.assertRaisesRegex(ValueError, "limit"):
            probe("rusthouse", "part", 5, limit=6)

    def test_detects_wrong_order(self):
        with self.assertRaisesRegex(ValueError, "differs"):
            probe("rusthouse", "part", 3, limit=1,
                  execute=lambda *a, **k: SimpleNamespace(returncode=0, stdout=b"9\n", stderr=b""))

    def test_cli_forwards_configured_topk_deadline(self):
        from unittest.mock import patch
        from bench.scale.topk_probe import main
        with patch("bench.scale.topk_probe.probe", return_value={"results": []}) as call:
            self.assertEqual(main(["--part", "part", "--binary", "rusthouse",
                                   "--rows", "5", "--timeout", "42"]), 0)
            self.assertEqual(call.call_args.kwargs["timeout"], 42.0)




class SpoolingTopKProbeTests(unittest.TestCase):
    def test_real_cli_path_reads_only_expected_prefix(self):
        def runaway(_command, **kwargs):
            self.assertIn("stdout", kwargs)
            self.assertNotIn("capture_output", kwargs)
            kwargs["stdout"].write(b"9\n" * 50_000)
            return SimpleNamespace(returncode=0, stderr=b"")
        with patch("bench.scale.topk_probe.subprocess.run", side_effect=runaway):
            with self.assertRaisesRegex(ValueError, "differs"):
                probe("rusthouse", "part", 1_000_000, limit=1)

class BoundedTopKFailureTests(unittest.TestCase):
    def test_failed_injected_cli_keeps_only_stderr_tail(self):
        def failing(*_args, **_kwargs):
            return SimpleNamespace(returncode=2, stdout=b"",
                                   stderr=b"a" * 10_000 + b"final failure")

        with self.assertRaises(RuntimeError) as raised:
            probe("rusthouse", "part", 5, execute=failing)
        self.assertLess(len(str(raised.exception)), 750)
        self.assertIn("final failure", str(raised.exception))

    def test_default_subprocess_stderr_is_spooled_and_bounded(self):
        def failing(_command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            self.assertTrue(hasattr(kwargs["stderr"], "write"))
            kwargs["stderr"].write(b"a" * 10_000 + b"last error")
            return SimpleNamespace(returncode=1)

        with patch("bench.scale.topk_probe.subprocess.run", side_effect=failing):
            with self.assertRaisesRegex(RuntimeError, "last error") as raised:
                probe("rusthouse", "part", 5)
        self.assertLess(len(str(raised.exception)), 750)


if __name__ == "__main__":
    unittest.main()

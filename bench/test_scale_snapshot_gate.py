import json
import unittest
from unittest.mock import patch
from types import SimpleNamespace

from bench.scale.snapshot_gate import inspect_snapshot


class SnapshotGateTests(unittest.TestCase):
    def test_failed_default_inspection_spools_stderr(self):
        def fail(_command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            kwargs["stderr"].write(b"x" * 100_000 + b"reason")
            return SimpleNamespace(returncode=1)
        with patch("bench.scale.snapshot_gate.subprocess.run", side_effect=fail):
            with self.assertRaisesRegex(RuntimeError, "reason") as failure:
                inspect_snapshot("rusthouse", "part", min_rows=1)
        self.assertLess(len(str(failure.exception)), 700)


    def test_posix_inspection_child_has_online_output_limit(self):
        import os
        if os.name != "posix":
            self.skipTest("POSIX file-size limit required")

        def run(command, **kwargs):
            self.assertTrue(callable(kwargs.get("preexec_fn")))
            kwargs["stdout"].write(
                b'{"rows":100,"parts":1,"columns":[{"name":"id","type":"UInt64"}]}'
            )
            return SimpleNamespace(returncode=0)

        with patch("bench.scale.snapshot_gate.subprocess.run", side_effect=run):
            self.assertEqual(inspect_snapshot("binary", "part", min_rows=1)["physical_rows"], 100)

    def test_accepts_billion_physical_rows(self):
        def run(command, **kwargs):
            self.assertEqual(command[1], "inspect-snapshot")
            return SimpleNamespace(returncode=0, stdout=json.dumps({
                "rows": 1_000_000_001, "parts": 10000,
                "columns": [{"name": "id", "type": "UInt64"}]
            }).encode(), stderr=b"")
        report = inspect_snapshot("rusthouse", "/frozen/parts", run=run)
        self.assertEqual(report["physical_rows"], 1_000_000_001)
        self.assertFalse(report["physical_scan_timed"])

    def test_max_parts_prevents_oversharded_benchmark_snapshots(self):
        def run(command, **kwargs):
            return SimpleNamespace(returncode=0, stdout=json.dumps({
                "rows": 1_000_000_001, "parts": 10000,
                "columns": [{"name": "id", "type": "UInt64"}]
            }).encode(), stderr=b"")
        with self.assertRaisesRegex(ValueError, "above max_parts"):
            inspect_snapshot("rusthouse", "parts", max_parts=9999, run=run)
        valid = inspect_snapshot("rusthouse", "parts", max_parts=10000, run=run)
        self.assertEqual(valid["maximum_parts"], 10000)
        with self.assertRaisesRegex(ValueError, "max_parts"):
            inspect_snapshot("rusthouse", "parts", max_parts=16385, run=run)

    def test_rejects_small_or_invalid_metadata(self):
        def run(command, **kwargs):
            return SimpleNamespace(returncode=0, stdout=b'{"rows":10,"parts":1,"columns":[1]}', stderr=b"")
        with self.assertRaisesRegex(ValueError, "below required"):
            inspect_snapshot("rusthouse", "parts", run=run)
        with self.assertRaises(ValueError):
            inspect_snapshot("rusthouse", "parts", min_rows=0, run=run)

    def test_rejects_high_row_counts_with_invalid_column_metadata(self):
        for columns in ([1], [{"name": "", "type": "UInt64"}],
                        [{"name": "id", "type": "UInt64"}] * 2):
            def run(command, **kwargs):
                return SimpleNamespace(returncode=0, stdout=json.dumps({
                    "rows": 1_000_000_001, "parts": 10000, "columns": columns
                }).encode(), stderr=b"")
            with self.assertRaisesRegex(ValueError, "columns"):
                inspect_snapshot("rusthouse", "/frozen/parts", run=run)

    def test_rejects_snapshot_with_more_than_128_columns(self):
        columns = [{"name": f"field_{index}", "type": "UInt64"} for index in range(129)]
        payload = json.dumps({"rows": 100, "parts": 1, "columns": columns}).encode()
        with self.assertRaisesRegex(ValueError, "columns"):
            inspect_snapshot("rusthouse", "part", min_rows=1,
                             run=lambda *_a, **_k: SimpleNamespace(
                                 returncode=0, stdout=payload, stderr=b""))

    def test_failure_propagates(self):
        def run(command, **kwargs):
            return SimpleNamespace(returncode=1, stdout=b"", stderr=b"overlapping")
        with self.assertRaisesRegex(RuntimeError, "overlapping"):
            inspect_snapshot("rusthouse", "parts", run=run)

    def test_rejects_nonobject_snapshot_json(self):
        for payload in ([], None, 0, "unavailable"):
            def run(*_args, **_kwargs):
                return SimpleNamespace(returncode=0,
                                       stdout=json.dumps(payload).encode(), stderr=b"")
            with self.subTest(payload=payload), self.assertRaisesRegex(ValueError, "JSON object"):
                inspect_snapshot("binary", "part", min_rows=1, run=run)

    def test_rejects_invalid_snapshot_part_and_row_domains(self):
        from types import SimpleNamespace
        from bench.scale.snapshot_gate import inspect_snapshot
        import json
        for rows, parts in ((1, 16385), (1 << 64, 1), (True, 1)):
            payload = {"rows": rows, "parts": parts,
                       "columns": [{"name": "id", "type": "UInt64"}]}
            def mocked(*args, **kwargs):
                return SimpleNamespace(returncode=0, stdout=json.dumps(payload).encode(), stderr=b"")
            with self.assertRaisesRegex(ValueError, "invalid rows or parts"):
                inspect_snapshot("rusthouse", ".", min_rows=1, run=mocked)




class SnapshotTimeoutTests(unittest.TestCase):
    def test_custom_snapshot_inspection_timeout(self):
        def run(command, **kwargs):
            self.assertEqual(kwargs["timeout"], 2.5)
            return SimpleNamespace(returncode=0, stdout=json.dumps({
                "rows": 10, "parts": 1,
                "columns": [{"name": "id", "type": "UInt64"}]
            }).encode(), stderr=b"")
        self.assertEqual(inspect_snapshot("rusthouse", "parts", min_rows=10,
                                          timeout=2.5, run=run)["physical_rows"], 10)
        for invalid in (0, -1, float("inf"), float("nan"), True):
            with self.subTest(invalid=invalid), self.assertRaisesRegex(ValueError, "timeout"):
                inspect_snapshot("rusthouse", "parts", timeout=invalid, run=run)


class SnapshotMetadataBudgetTests(unittest.TestCase):
    def test_rejects_oversized_inspection_before_json_parse(self):
        payload = b" " * (2 * 1024 * 1024 + 1)
        def run(*_args, **_kwargs):
            return SimpleNamespace(returncode=0, stdout=payload, stderr=b"")
        with self.assertRaisesRegex(ValueError, "2 MiB"):
            inspect_snapshot("rusthouse", "part", run=run)

    def test_normal_metadata_stays_supported(self):
        result = inspect_snapshot(
            "rusthouse", "part", min_rows=1,
            run=lambda *_args, **_kwargs: SimpleNamespace(
                returncode=0,
                stdout=b'{"rows":1,"parts":1,"columns":[{"name":"id","type":"UInt64"}]}',
                stderr=b""))
        self.assertEqual(result["physical_rows"], 1)

if __name__ == "__main__":
    unittest.main()

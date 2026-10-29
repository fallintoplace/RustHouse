"""Snapshot physical gates reject ambiguous JSON metadata."""
import unittest
from types import SimpleNamespace

from bench.scale.snapshot_gate import inspect_snapshot


class SnapshotGateDuplicateTests(unittest.TestCase):
    def test_rejects_duplicate_rows_in_inspection(self):
        output = b'{"rows":1,"rows":1000000000,"parts":1,"columns":[{"name":"id","type":"UInt64"}]}'
        def fake_run(*args, **kwargs):
            return SimpleNamespace(returncode=0, stdout=output, stderr=b"")
        with self.assertRaisesRegex(ValueError, "duplicate snapshot JSON field"):
            inspect_snapshot("rusthouse", "/parts", min_rows=1, run=fake_run)

    def test_accepts_unambiguous_report(self):
        output = b'{"rows":5,"parts":1,"columns":[{"name":"id","type":"UInt64"}]}'
        def fake_run(*args, **kwargs):
            return SimpleNamespace(returncode=0, stdout=output, stderr=b"")
        report = inspect_snapshot("rusthouse", "/parts", min_rows=5, run=fake_run)
        self.assertEqual(report["physical_rows"], 5)

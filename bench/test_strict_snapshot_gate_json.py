"""Snapshot inspection JSON must follow the standard number grammar."""
import unittest
from types import SimpleNamespace

from bench.scale.snapshot_gate import inspect_snapshot


class StrictSnapshotJsonTests(unittest.TestCase):
    def test_rejects_nonfinite_extra_fields(self):
        for number in (b"NaN", b"Infinity", b"-Infinity"):
            payload = (b'{"rows":1,"parts":1,"columns":[{"name":"id","type":"UInt64"}],'
                       b'"extra":' + number + b'}')
            def run(*args, **kwargs):
                return SimpleNamespace(returncode=0, stdout=payload, stderr=b"")
            with self.subTest(number=number), self.assertRaisesRegex(ValueError, "JSON constant"):
                inspect_snapshot("bin", "part", min_rows=1, run=run)

    def test_accepts_valid_report(self):
        payload = b'{"rows":1,"parts":1,"columns":[{"name":"id","type":"UInt64"}]}'
        report = inspect_snapshot(
            "bin", "part", min_rows=1,
            run=lambda *args, **kwargs: SimpleNamespace(
                returncode=0, stdout=payload, stderr=b""))
        self.assertEqual(report["physical_rows"], 1)


if __name__ == "__main__":
    unittest.main()

import unittest
from types import SimpleNamespace

from bench.scale.snapshot_gate import inspect_snapshot


class StrictSnapshotJsonTests(unittest.TestCase):
    def test_rejects_nan_and_infinity_even_in_other_fields(self):
        for constant in ("NaN", "Infinity", "-Infinity"):
            payload = ('{"rows":12,"parts":1,"columns":'
                       '[{"name":"id","type":"UInt64"}],"extra":' + constant + '}')
            run = lambda *_args, **_kw: SimpleNamespace(
                returncode=0, stdout=payload.encode("ascii"), stderr=b"")
            with self.subTest(constant=constant):
                with self.assertRaisesRegex(ValueError, "nonstandard snapshot JSON"):
                    inspect_snapshot("binary", "part", min_rows=1, run=run)

    def test_valid_report_remains_accepted(self):
        payload = (b'{"rows":12,"parts":1,"columns":'
                   b'[{"name":"id","type":"UInt64"}]}')
        run = lambda *_args, **_kw: SimpleNamespace(
            returncode=0, stdout=payload, stderr=b"")
        self.assertEqual(inspect_snapshot("binary", "part", min_rows=1, run=run)["physical_rows"], 12)


if __name__ == "__main__":
    unittest.main()

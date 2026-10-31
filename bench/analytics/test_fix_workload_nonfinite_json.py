"""Workload manifests must use standard JSON numeric values."""
import tempfile
import unittest
from pathlib import Path

from suite_runner import load_workload


class WorkloadStrictJsonTests(unittest.TestCase):
    def test_rejects_nonstandard_numeric_tokens(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "workload.json"
            for value in ("NaN", "Infinity", "-Infinity", "1e10000", "-1e10000"):
                with self.subTest(value=value):
                    source.write_text(
                        '{"name":"small","queries":[{"name":"q1","sql":"SELECT 1"}],"extra":'
                        + value + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "nonstandard workload"):
                        load_workload(source)

    def test_valid_workload_manifest_still_loads(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "workload.json"
            source.write_text(
                '{"name":"small","queries":[{"name":"q1","sql":"SELECT 1"}]}',
                encoding="utf-8")
            self.assertEqual(load_workload(source)["name"], "small")


if __name__ == "__main__":
    unittest.main()

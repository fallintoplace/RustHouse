"""Tests for the scheduled benchmark artifact gate."""

import tempfile
import unittest
from pathlib import Path

from bench.__main__ import BENCHMARK_SUITES, benchmark_command
from bench.validate_benchmark import validate_manifest


def sample_manifest(directory):
    entries = []
    for suite in BENCHMARK_SUITES:
        suffix = "csv" if suite.startswith("parts-") else "txt"
        output = f"{suite}.{suffix}"
        error = f"{suite}.stderr.txt"
        (directory / output).write_text("measurement\n", encoding="utf-8")
        (directory / error).write_text("", encoding="utf-8")
        entries.append({
            "suite": suite, "status": "passed", "exit_code": 0,
            "command": benchmark_command(suite, 65536, 7, 413),
            "elapsed_seconds": 1.25, "stdout_file": output, "stderr_file": error,
        })
    return {
        "schema_version": 1, "rows": 65536, "samples": 7, "stations": 413,
        "results": entries,
    }


def verify(directory, manifest):
    return validate_manifest(directory, manifest, rows=65536, samples=7, stations=413)


class ValidateBenchmarkManifestTests(unittest.TestCase):
    def test_rejects_oversized_manifest_before_parsing(self):
        from unittest.mock import patch
        from bench.validate_benchmark import load_manifest
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            path.write_bytes(b"X" * 17)
            with patch("bench.validate_benchmark.MAX_MANIFEST_BYTES", 16):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    load_manifest(path)


    def test_rejects_duplicate_manifest_fields(self):
        from bench.validate_benchmark import load_manifest
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            path.write_text('{"results":[{"status":"passed","status":"failed"}]}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate benchmark manifest key"):
                load_manifest(path)

    def test_rejects_manifest_swapped_between_stat_and_open(self):
        import os
        from unittest.mock import patch
        from bench.validate_benchmark import load_manifest

        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            replacement = Path(root) / "replacement.json"
            path.write_text('{"schema_version":1}', encoding="utf-8")
            replacement.write_text('{"schema_version":2}', encoding="utf-8")
            original_open = os.open

            def swap_before_open(filename, flags):
                path.unlink()
                replacement.rename(path)
                return original_open(filename, flags)

            with patch("bench.validate_benchmark.os.open", side_effect=swap_before_open):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    load_manifest(path)

    def test_rejects_manifest_replaced_during_read(self):
        import os
        from unittest.mock import patch
        from bench.validate_benchmark import load_manifest

        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "manifest.json"
            replacement = Path(root) / "replacement.json"
            path.write_text('{"schema_version":1}', encoding="utf-8")
            replacement.write_text('{"schema_version":2}', encoding="utf-8")
            original_fstat = os.fstat
            calls = 0

            def mutate_before_finish(fd):
                nonlocal calls
                calls += 1
                if calls == 2:
                    path.unlink()
                    replacement.rename(path)
                return original_fstat(fd)

            with patch("bench.validate_benchmark.os.fstat", side_effect=mutate_before_finish):
                with self.assertRaisesRegex(ValueError, "changed while reading"):
                    load_manifest(path)

    def test_rejects_boolean_schema_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            report = sample_manifest(directory)
            report["schema_version"] = True
            with self.assertRaisesRegex(ValueError, "unsupported benchmark"):
                verify(directory, report)

    def test_accepts_four_complete_suites(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            self.assertEqual(verify(directory, sample_manifest(directory)), 4)

    def test_rejects_missing_suite(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            report = sample_manifest(directory)
            report["results"].pop()
            with self.assertRaisesRegex(ValueError, "suite records"):
                verify(directory, report)

    def test_rejects_duplicate_suite(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            report = sample_manifest(directory)
            report["results"][1]["suite"] = report["results"][0]["suite"]
            with self.assertRaisesRegex(ValueError, "duplicate"):
                verify(directory, report)

    def test_rejects_failed_suite(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            report = sample_manifest(directory)
            report["results"][0]["status"] = "failed"
            with self.assertRaisesRegex(ValueError, "did not pass"):
                verify(directory, report)

    def test_rejects_missing_measurements(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            report = sample_manifest(directory)
            (directory / report["results"][0]["stdout_file"]).unlink()
            with self.assertRaisesRegex(ValueError, "missing or empty"):
                verify(directory, report)

    def test_rejects_empty_measurements(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            report = sample_manifest(directory)
            (directory / report["results"][0]["stdout_file"]).write_text("")
            with self.assertRaisesRegex(ValueError, "missing or empty"):
                verify(directory, report)

    def test_rejects_changed_benchmark_command(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            report = sample_manifest(directory)
            report["results"][0]["command"].append("--bogus")
            with self.assertRaisesRegex(ValueError, "command differs"):
                verify(directory, report)

    def test_rejects_missing_diagnostics(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            report = sample_manifest(directory)
            (directory / report["results"][0]["stderr_file"]).unlink()
            with self.assertRaisesRegex(ValueError, "missing benchmark diagnostics"):
                verify(directory, report)


if __name__ == "__main__":
    unittest.main()

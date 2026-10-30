"""Unavailable version commands cannot hang benchmark metadata capture."""
import importlib
import subprocess
import unittest
from unittest import mock

from bench import __main__ as bench_cli


class VersionMetadataTimeoutTests(unittest.TestCase):
    def test_entrypoint_version_probe_has_timeout(self):
        with mock.patch.object(bench_cli.subprocess, "run",
                               side_effect=subprocess.TimeoutExpired("git", 5)) as run:
            self.assertIsNone(bench_cli.optional_version(["git", "--version"]))
            self.assertEqual(run.call_args.kwargs["timeout"], 5)

    def test_latency_version_probe_has_timeout(self):
        latency = importlib.import_module("bench.latency.run")
        with mock.patch.object(latency.subprocess, "run",
                               side_effect=subprocess.TimeoutExpired("rustc", 5)) as run:
            self.assertIsNone(latency.optional_version(["rustc", "--version"]))
            self.assertEqual(run.call_args.kwargs["timeout"], 5)

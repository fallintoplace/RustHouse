"""One entry point for RustHouse checks, Python harness tests, and Rust benchmarks.

Run from the repository root with: python3 -m bench --help
"""

import argparse
import datetime
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PYTHON_SUITES = ("bench", "bench/analytics", "bench/clickbench", "bench/interop", "bench/joins", "bench/latency", "bench/measurements", "bench/ssb", "bench/tpch")
BENCHMARK_SUITES = ("queries", "parts-sorted", "parts-shuffled", "grouped")
WORKLOADS = {
    "analytics": "bench/analytics/run.py",
    "analytics-suite": "bench/analytics/suite_runner.py",
    "clickbench": "bench/clickbench/run.py",
    "joins": "bench/joins/run.py",
    "latency": "bench/latency/run.py",
    "measurements": "bench/measurements/ingest.py",
    "ssb": "bench/ssb/run.py",
    "tpch": "bench/tpch/run.py",
    "physical-scan": "bench/scale/physical_scan.py",
}


def benchmark_command(suite, rows, samples, stations):
    """Build the exact Cargo command for a supported release-mode benchmark."""
    targets = {
        "queries": ("queries", []),
        "parts-sorted": ("parts", ["--layout", "sorted", "--csv"]),
        "parts-shuffled": ("parts", ["--layout", "shuffled", "--csv"]),
        "grouped": ("grouped", ["--stations", str(stations)]),
    }
    target, options = targets[suite]
    return [
        "cargo", "bench", "-p", "rusthouse-query", "--bench", target,
        "--locked", "--", "--rows", str(rows), "--samples", str(samples), *options,
    ]


def run_command(name, command):
    print(f"\n[{name}] {' '.join(command)}", flush=True)
    try:
        return subprocess.run(command, cwd=ROOT, check=False).returncode
    except OSError as error:
        print(f"[{name}] {error}", file=sys.stderr)
        return 1


def python_tests():
    """Discover each workload independently to avoid duplicate test_run module names."""
    failed = False
    for directory in PYTHON_SUITES:
        command = [
            sys.executable, "-m", "unittest", "discover", "-s", directory,
            "-p", "test_*.py",
        ]
        failed |= run_command(directory, command) != 0
    return int(failed)


def check():
    """Run all local correctness checks without running noisy performance timings."""
    commands = (
        ("format", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"]),
        ("rust-tests", ["cargo", "test", "--workspace", "--locked"]),
        ("build", ["cargo", "build", "--workspace", "--locked"]),
    )
    failed = False
    for name, command in commands:
        failed |= run_command(name, command) != 0
    failed |= python_tests() != 0
    return int(failed)


def optional_version(command):
    try:
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, check=False)
    except OSError:
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def benchmark(suites, rows, samples, stations, output_dir):
    """Run verified benches and preserve machine metadata and failure outputs."""
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    manifest = {
        "schema_version": 1,
        "started_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "revision": os.environ.get("GITHUB_SHA") or optional_version(["git", "rev-parse", "HEAD"]),
        "rustc": optional_version(["rustc", "--version"]),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "cpu_count": os.cpu_count(),
        "rows": rows,
        "samples": samples,
        "stations": stations if "grouped" in suites else None,
        "results": [],
    }
    failed = False
    try:
        for suite in suites:
            command = benchmark_command(suite, rows, samples, stations)
            extension = "csv" if suite.startswith("parts-") else "txt"
            stdout_name = f"{suite}.{extension}"
            stderr_name = f"{suite}.stderr.txt"
            print(f"\n[{suite}] {' '.join(command)}", flush=True)
            started = time.perf_counter()
            try:
                completed = subprocess.run(
                    command, cwd=ROOT, capture_output=True, text=True, check=False,
                )
                returncode, stdout, stderr = completed.returncode, completed.stdout, completed.stderr
            except OSError as error:
                returncode, stdout, stderr = 1, "", str(error) + "\n"
            elapsed = round(time.perf_counter() - started, 6)
            (output_dir / stdout_name).write_text(stdout, encoding="utf-8")
            (output_dir / stderr_name).write_text(stderr, encoding="utf-8")
            if stdout:
                sys.stdout.write(stdout)
            if stderr:
                sys.stderr.write(stderr)
            failed |= returncode != 0
            manifest["results"].append({
                "suite": suite,
                "command": command,
                "status": "failed" if returncode else "passed",
                "exit_code": returncode,
                "elapsed_seconds": elapsed,
                "stdout_file": stdout_name,
                "stderr_file": stderr_name,
            })
    finally:
        (output_dir / "manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
    return int(failed)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("check", help="Run Rust formatting, lint, tests, build, and Python tests")
    commands.add_parser("python-tests", help="Run all Python benchmark harness tests")
    workload = commands.add_parser("workload", help="Run an existing Python SQL or import workload")
    workload.add_argument("name", choices=WORKLOADS)
    workload.add_argument("args", nargs=argparse.REMAINDER, help="Arguments to the workload after --")
    bench = commands.add_parser("benchmark", help="Run all or selected release-mode Rust benchmarks")
    bench.add_argument("--suite", choices=BENCHMARK_SUITES, action="append", help="Repeat to select suites (default: all)")
    bench.add_argument("--rows", type=int, default=65536)
    bench.add_argument("--samples", type=int, default=7)
    bench.add_argument("--stations", type=int, default=413)
    bench.add_argument("--output-dir", type=Path, default=ROOT / "bench" / "results")
    args = parser.parse_args(argv)
    if args.command == "check":
        return check()
    if args.command == "python-tests":
        return python_tests()
    if args.command == "workload":
        forwarded = args.args[1:] if args.args[:1] == ["--"] else args.args
        # Execute analytics-suite as a package so bench.common resolves on CI.
        if args.name == "analytics-suite":
            return run_command(args.name, [sys.executable, "-m", "bench.analytics.suite_runner", *forwarded])
        return run_command(args.name, [sys.executable, str(ROOT / WORKLOADS[args.name]), *forwarded])
    suites = tuple(dict.fromkeys(args.suite or BENCHMARK_SUITES))
    if not 1 <= args.rows <= 1_000_000:
        parser.error("--rows must be in 1..1000000")
    if args.samples < 1 or args.samples % 2 != 1:
        parser.error("--samples must be a positive odd number")
    if not 1 <= args.stations <= 100_000 or ("grouped" in suites and args.stations > args.rows):
        parser.error("--stations must be in 1..100000 and cannot exceed --rows for grouped")
    return benchmark(suites, args.rows, args.samples, args.stations, args.output_dir)


if __name__ == "__main__":
    raise SystemExit(main())


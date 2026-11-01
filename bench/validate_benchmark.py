"""Verify scheduled Rust benchmark results before archiving artifacts."""

import argparse
import json
import math
import sys
from pathlib import Path

MAX_MANIFEST_BYTES = 2 * 1024 * 1024


def reject_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate benchmark manifest key: {key}")
        result[key] = value
    return result


def _reject_nonstandard_number(token):
    raise ValueError(f"nonstandard scheduled benchmark manifest JSON number: {token}")


def load_manifest(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError("manifest must be a regular non-symlinked file")
    with path.open("rb") as source:
        payload = source.read(MAX_MANIFEST_BYTES + 1)
    if len(payload) > MAX_MANIFEST_BYTES:
        raise ValueError("manifest exceeds 2 MiB")
    return json.loads(payload, object_pairs_hook=reject_duplicate_keys,
                      parse_constant=_reject_nonstandard_number)


from bench.__main__ import BENCHMARK_SUITES, benchmark_command


def validate_manifest(directory, manifest, *, rows, samples, stations):
    if (not isinstance(manifest, dict) or
            type(manifest.get("schema_version")) is not int or
            manifest["schema_version"] != 1):
        raise ValueError("unsupported benchmark manifest")
    for key, expected in (("rows", rows), ("samples", samples), ("stations", stations)):
        if type(manifest.get(key)) is not int or manifest[key] != expected:
            raise ValueError(f"expected {key}={expected}")
    results = manifest.get("results")
    if not isinstance(results, list) or len(results) != len(BENCHMARK_SUITES):
        raise ValueError(f"expected {len(BENCHMARK_SUITES)} benchmark suite records")
    seen = set()
    for record in results:
        if not isinstance(record, dict):
            raise ValueError("invalid benchmark suite record")
        suite = record.get("suite")
        if suite not in BENCHMARK_SUITES or suite in seen:
            raise ValueError(f"unknown or duplicate benchmark suite: {suite!r}")
        seen.add(suite)
        if record.get("status") != "passed" or type(record.get("exit_code")) is not int or record["exit_code"] != 0:
            raise ValueError(f"benchmark suite did not pass: {suite}")
        if record.get("command") != benchmark_command(suite, rows, samples, stations):
            raise ValueError(f"benchmark command differs from expected configuration: {suite}")
        elapsed = record.get("elapsed_seconds")
        if type(elapsed) not in (int, float) or not math.isfinite(elapsed) or elapsed < 0:
            raise ValueError(f"invalid benchmark runtime: {suite}")
        suffix = "csv" if suite.startswith("parts-") else "txt"
        output = f"{suite}.{suffix}"
        error = f"{suite}.stderr.txt"
        if record.get("stdout_file") != output or record.get("stderr_file") != error:
            raise ValueError(f"unexpected output filename for benchmark suite: {suite}")
        stdout_path = directory / output
        stderr_path = directory / error
        if stdout_path.is_symlink() or not stdout_path.is_file() or stdout_path.stat().st_size == 0:
            raise ValueError(f"missing or empty benchmark measurements: {output}")
        if stderr_path.is_symlink() or not stderr_path.is_file():
            raise ValueError(f"missing benchmark diagnostics: {error}")
    if seen != set(BENCHMARK_SUITES):
        raise ValueError("benchmark manifest does not cover all configured suites")
    return len(seen)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--samples", type=int, required=True)
    parser.add_argument("--stations", type=int, required=True)
    args = parser.parse_args(argv)
    try:
        manifest = load_manifest(args.output_dir / "manifest.json")
        count = validate_manifest(
            args.output_dir, manifest, rows=args.rows, samples=args.samples, stations=args.stations
        )
    except (OSError, ValueError) as error:
        print(f"benchmark artifact validation failed: {error}", file=sys.stderr)
        return 1
    print(f"benchmark artifacts validated: {count} complete suites")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

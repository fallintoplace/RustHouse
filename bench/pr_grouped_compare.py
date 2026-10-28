#!/usr/bin/env python3
"""Compare validated GROUP BY medians for base and PR on the same runner."""
import argparse
import math
import re
import sys
from pathlib import Path

CASES = ("min", "avg", "max")
HEADER = re.compile(
    r"RustHouse grouped scans: (\d+) rows, (\d+) keys, (\d+) samples"
)
MEASURE = re.compile(
    r"(min|avg|max)\s+(\d+\.\d+)\s+(\d+\.\d+)\s+(\d+\.\d+)"
)


def parse_benchmark(text):
    settings = None
    medians = {}
    for line in text.splitlines():
        line = line.strip()
        header = HEADER.fullmatch(line)
        measurement = MEASURE.fullmatch(line)
        if header:
            if settings is not None:
                raise ValueError("duplicate grouped benchmark header")
            settings = tuple(map(int, header.groups()))
        elif measurement:
            key, median, _p90, _rate = measurement.groups()
            if key in medians:
                raise ValueError(f"duplicate grouped case: {key}")
            value = float(median)
            if not math.isfinite(value) or value <= 0:
                raise ValueError("grouped medians must be positive")
            medians[key] = value
    if settings is None:
        raise ValueError("missing grouped benchmark settings")
    if set(medians) != set(CASES):
        raise ValueError("missing grouped benchmark workloads")
    if settings[0] <= 0 or settings[1] <= 0 or settings[2] <= 0:
        raise ValueError("invalid benchmark settings")
    return settings, medians


def compare_text(baseline, candidate):
    base_settings, before = parse_benchmark(baseline)
    head_settings, after = parse_benchmark(candidate)
    if base_settings != head_settings:
        raise ValueError("base and PR GROUP BY settings differ")
    rows, keys, samples = base_settings
    lines = [
        "## RustHouse GROUP BY performance: base vs PR",
        "",
        f"Verified grouped SQL with {rows:,} rows, {keys:,} groups, "
        f"{samples} timed samples per aggregate.",
        "",
        "| Aggregate | Base median (ms) | PR median (ms) | Change |",
        "| --- | ---: | ---: | ---: |",
    ]
    for aggregate in CASES:
        previous = before[aggregate]
        current = after[aggregate]
        change = 100 * (current / previous - 1)
        lines.append(
            f"| {aggregate} | {previous:.3f} | {current:.3f} | {change:+.1f}% |"
        )
    lines.extend([
        "",
        "Negative change means faster. Same GitHub Actions runner, but CPU "
        "contention may still affect timings. Informational, not a gate.",
        "",
    ])
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        report = compare_text(
            args.baseline.read_text(encoding="utf-8"),
            args.candidate.read_text(encoding="utf-8"),
        )
        args.summary.write_text(report, encoding="utf-8")
    except (OSError, ValueError) as error:
        print(f"GROUP BY comparison failed: {error}", file=sys.stderr)
        return 1
    print(report, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

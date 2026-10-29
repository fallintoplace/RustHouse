"""Summarize same-runner base-versus-PR numbers() benchmark measurements.

The Rust harness checks output correctness before reporting each median.
"""

import argparse
import math
import re
import sys
from pathlib import Path

WORKLOADS = ("filtered count", "filtered sum", "filtered top-K")
METADATA = re.compile(
    r"RustHouse numbers\(\) benchmarks: (\d+) input rows, (\d+) samples, (\d+) warmups"
)
MEASUREMENT = re.compile(r"(filtered (?:count|sum|top-K))\s+(\d+\.\d+)\s+(\d+\.\d+)")


def parse_benchmark(text):
    """Require the same benchmark cases and settings on both revisions."""
    settings = None
    medians = {}
    for line in text.splitlines():
        line = line.strip()
        if match := METADATA.fullmatch(line):
            if settings is not None:
                raise ValueError("duplicate benchmark settings")
            settings = tuple(map(int, match.groups()))
        elif match := MEASUREMENT.fullmatch(line):
            name, median, _throughput = match.groups()
            if name in medians:
                raise ValueError(f"duplicate workload: {name}")
            value = float(median)
            rate = float(_throughput)
            if not math.isfinite(value) or value < 0:
                raise ValueError(f"invalid benchmark median: {name}")
            if not math.isfinite(rate) or rate <= 0:
                raise ValueError(f"invalid benchmark throughput: {name}")
            medians[name] = value
    if settings is None:
        raise ValueError("missing numbers() benchmark settings")
    if settings[0] < 1 or settings[1] < 1:
        raise ValueError("benchmark rows and samples must be positive")
    if set(medians) != set(WORKLOADS):
        raise ValueError(f"missing or unexpected workloads: {sorted(set(WORKLOADS) ^ set(medians))}")
    return settings, medians


def compare_text(baseline, candidate):
    base_settings, before = parse_benchmark(baseline)
    head_settings, after = parse_benchmark(candidate)
    if base_settings != head_settings:
        raise ValueError("baseline and PR benchmark settings do not match")
    rows, samples, warmups = base_settings
    result = [
        "## RustHouse performance: base vs PR",
        "",
        f"Verified numbers() SQL execution, {rows:,} input rows, "
        f"{samples} samples, {warmups} warmups per case.",
        "",
        "| Workload | Base median (ms) | PR median (ms) | PR latency change |",
        "| --- | ---: | ---: | ---: |",
    ]
    for name in WORKLOADS:
        base_ms, head_ms = before[name], after[name]
        if base_ms <= 0 or head_ms <= 0:
            change = "N/A (below printed precision)"
        else:
            change = f"{(head_ms / base_ms - 1) * 100:+.1f}%"
        result.append(f"| {name} | {base_ms:.6f} | {head_ms:.6f} | {change} |")
    result.extend([
        "",
        "Negative latency change means faster. Both revisions ran on the same "
        "GitHub Actions runner, but CPU contention and thermal variation can "
        "still affect results. Timings are informational, not a required CI gate.",
        "",
    ])
    return "\n".join(result)


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
        print(f"performance comparison failed: {error}", file=sys.stderr)
        return 1
    print(report, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

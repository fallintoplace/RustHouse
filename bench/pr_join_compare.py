"""Compare verified physical JOIN medians from the same GitHub Actions runner."""

import argparse
import math
import re
import sys
from pathlib import Path

SETTINGS = re.compile(
    r"RustHouse hash JOIN benchmark: (\d+) probe rows, (\d+) build rows, "
    r"(\d+) samples, (\d+) warmups"
)
SAMPLE = re.compile(
    r"inner equi-join (\d+\.\d+) (\d+\.\d+) (\d+\.\d+)"
)


def parse_benchmark(output):
    settings = None
    measurement = None
    for line in output.splitlines():
        line = line.strip()
        if match := SETTINGS.fullmatch(line):
            if settings is not None:
                raise ValueError("duplicate physical JOIN benchmark settings")
            settings = tuple(map(int, match.groups()))
        elif match := SAMPLE.fullmatch(line):
            if measurement is not None:
                raise ValueError("duplicate physical JOIN benchmark sample")
            measurement = tuple(map(float, match.groups()))
    if settings is None or measurement is None:
        raise ValueError("missing physical JOIN benchmark metadata or sample")
    rows, build, samples, warmups = settings
    median, p90, throughput = measurement
    if not (100 <= rows <= 65_536 and 1 <= build <= rows and samples > 0
            and samples % 2 == 1 and warmups >= 0):
        raise ValueError("invalid physical JOIN fixture settings")
    if not all(math.isfinite(value) and value > 0 for value in measurement):
        raise ValueError("physical JOIN samples must be finite and positive")
    if p90 < median:
        raise ValueError("p90 cannot be below the median")
    return settings, measurement


def compare_text(baseline, candidate):
    before_settings, (base, base_p90, _) = parse_benchmark(baseline)
    after_settings, (head, head_p90, _) = parse_benchmark(candidate)
    if before_settings != after_settings:
        raise ValueError("base and PR physical JOIN fixtures differ")
    rows, build, samples, warmups = before_settings
    change = (head / base - 1) * 100
    speedup = base / head
    return (
        "## Physical hash JOIN: base vs PR\n\n"
        f"Verified on-disk equi-join, {rows:,} probe rows, {build:,} build rows; "
        f"{samples} samples and {warmups} warmups per revision.\n\n"
        "| Metric | Base | PR |\n"
        "| --- | ---: | ---: |\n"
        f"| Median (ms) | {base:.6f} | {head:.6f} |\n"
        f"| p90 (ms) | {base_p90:.6f} | {head_p90:.6f} |\n"
        f"\nPR median latency change: **{change:+.1f}%** "
        f"({speedup:.3f}x base/PR speed ratio). Negative is faster.\n\n"
        "Results include immutable-part scans, hash join and result rendering, "
        "not fixture creation. Same runner, but not isolated CPU load. "
        "This is informational and is not a ClickHouse comparison.\n"
    )


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
        print(f"JOIN benchmark comparison failed: {error}", file=sys.stderr)
        return 1
    print(report, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

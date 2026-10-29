"""Summarize correctness-verified persisted wide-part scan benchmarks for a PR."""

import argparse
import csv
import io
import math
from pathlib import Path

FIELDS = ("workload", "median_ms", "p90_ms", "million_logical_rows_per_second")
CASES = (
    "count 100%", "count 50%", "count 10%", "count 1%",
    "filtered sum", "point filter", "large OFFSET", "filtered top-K",
)


def parse_benchmark(payload):
    lines = payload.splitlines()
    header = ",".join(FIELDS)
    try:
        start = lines.index(header)
    except ValueError as error:
        raise ValueError("missing persisted-parts benchmark CSV header") from error
    reader = csv.DictReader(io.StringIO("\n".join(lines[start:])))
    if tuple(reader.fieldnames or ()) != FIELDS:
        raise ValueError("unexpected persisted-parts columns")
    cases = {}
    for record in reader:
        name = record.get("workload")
        if name not in CASES or name in cases or None in record:
            raise ValueError("unexpected or duplicate persisted-parts workload")
        try:
            median, p90, throughput = (
                float(record[key]) for key in FIELDS[1:]
            )
        except (TypeError, ValueError) as error:
            raise ValueError(f"invalid timing for {name}") from error
        if not all(math.isfinite(value) and value > 0
                   for value in (median, p90, throughput)):
            raise ValueError(f"non-positive or non-finite timing for {name}")
        if p90 < median:
            raise ValueError(f"p90 below median for {name}")
        cases[name] = (median, p90)
    if set(cases) != set(CASES):
        raise ValueError("missing persisted-parts workloads")
    return cases


def compare_text(baseline, candidate):
    before = parse_benchmark(baseline)
    after = parse_benchmark(candidate)
    lines = [
        "## Persisted wide-part SQL: base vs PR", "",
        "Same runner, 32,768 stored rows, five verified samples per query.",
        "Warm OS page cache is possible. Informational, not a CI speed gate.", "",
        "| Workload | Base median (ms) | PR median (ms) | Change |",
        "| --- | ---: | ---: | ---: |",
    ]
    for name in CASES:
        previous, _ = before[name]
        current, _ = after[name]
        change = 100 * (current / previous - 1)
        label = f"{change:+.1f}%" if math.isfinite(change) else "N/A (ratio overflow)"
        lines.append(f"| {name} | {previous:.3f} | {current:.3f} | {label} |")
    lines.extend(["", "Negative change means faster. Re-run on a dedicated machine "
                  "before claiming a performance win.", ""])
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--summary", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        summary = compare_text(
            args.baseline.read_text(encoding="utf-8"),
            args.candidate.read_text(encoding="utf-8"),
        )
        args.summary.write_text(summary, encoding="utf-8")
    except (OSError, ValueError) as error:
        parser.exit(1, f"persisted-parts comparison failed: {error}\n")
    print(summary, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

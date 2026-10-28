"""Compare verified persisted-scan reports with explicit relative regression budgets."""
import argparse
import json
import math
from pathlib import Path

CASES = ("count_even", "sum_even")


def compare(baseline, candidate, *, max_slowdown_percent=10):
    if not math.isfinite(max_slowdown_percent) or max_slowdown_percent < 0:
        raise ValueError("max_slowdown_percent must be finite and nonnegative")
    expected = "rusthouse-persisted-wide-part-full-scans"
    if baseline.get("workload") != expected or candidate.get("workload") != expected:
        raise ValueError("reports must be persisted physical scan workloads")
    for field in ("runs", "warmups", "platform", "cpu_count", "source", "note"):
        old, new = baseline.get(field), candidate.get(field)
        if (old is None) != (new is None) or (old is not None and old != new):
            raise ValueError(f"physical benchmark {field} settings do not match")
    rows = baseline.get("rows_written_and_verified")
    if type(rows) is not int or rows < 1 or candidate.get("rows_written_and_verified") != rows:
        raise ValueError("reports must cover the same positive physical row count")
    cases = []
    for name in CASES:
        a, b = baseline.get(name), candidate.get(name)
        if not isinstance(a, dict) or not isinstance(b, dict):
            raise ValueError(f"missing {name} results")
        if a.get("verified") is not True or b.get("verified") is not True:
            raise ValueError(f"{name} must have verified reference output in both reports")
        if a.get("query") != b.get("query") or a.get("expected") != b.get("expected"):
            raise ValueError(f"{name} is not the same query and oracle")
        old, new = a.get("median_seconds"), b.get("median_seconds")
        if not all(type(x) in (int, float) and math.isfinite(x) and x > 0 for x in (old, new)):
            raise ValueError(f"{name} has invalid median timings")
        slowdown = 100 * (new / old - 1)
        cases.append({"name": name, "baseline_median_seconds": old,
                      "candidate_median_seconds": new, "slowdown_percent": slowdown,
                      "within_budget": slowdown <= max_slowdown_percent})
    return {"workload": "persisted-scan-regression-comparison",
            "rows": rows, "max_slowdown_percent": max_slowdown_percent,
            "all_within_budget": all(case["within_budget"] for case in cases),
            "cases": cases, "shared_ci_runners_are_noisy": True}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--max-slowdown-percent", type=float, default=10)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = compare(json.loads(args.baseline.read_text(encoding="utf-8")),
                         json.loads(args.candidate.read_text(encoding="utf-8")),
                         max_slowdown_percent=args.max_slowdown_percent)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, ValueError) as error:
        parser.exit(1, f"physical scan comparison failed: {error}\n")
    print(rendered, end="")
    return 0 if report["all_within_budget"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

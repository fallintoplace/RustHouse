"""Compare verified persisted-scan reports with explicit relative regression budgets."""
import argparse
import json
import math
import statistics
from pathlib import Path

from bench.scale.physical_scan import COUNT_SQL, SUM_SQL, even_reference

CASES = ("count_even", "sum_even")


def compare(baseline, candidate, *, max_slowdown_percent=10,
            max_tail_slowdown_percent=None):
    if (type(max_slowdown_percent) not in (int, float)
            or not math.isfinite(max_slowdown_percent) or max_slowdown_percent < 0):
        raise ValueError("max_slowdown_percent must be finite and nonnegative")
    if not isinstance(baseline, dict) or not isinstance(candidate, dict):
        raise ValueError("comparison reports must be JSON objects")
    if max_tail_slowdown_percent is not None and (
            type(max_tail_slowdown_percent) not in (int, float)
            or not math.isfinite(max_tail_slowdown_percent)
            or max_tail_slowdown_percent < 0):
        raise ValueError("max_tail_slowdown_percent must be finite and nonnegative")
    expected = "rusthouse-persisted-wide-part-full-scans"
    if baseline.get("workload") != expected or candidate.get("workload") != expected:
        raise ValueError("reports must be persisted physical scan workloads")
    for field in ("runs", "warmups", "platform", "cpu_count", "source", "note"):
        old, new = baseline.get(field), candidate.get(field)
        if (old is None) != (new is None) or (old is not None and old != new):
            raise ValueError(f"physical benchmark {field} settings do not match")
    rows = baseline.get("rows_written_and_verified")
    candidate_rows = candidate.get("rows_written_and_verified")
    if (type(rows) is not int or rows < 1
            or type(candidate_rows) is not int or candidate_rows != rows):
        raise ValueError("reports must cover the same positive physical row count")
    count, total = even_reference(rows)
    expected = {"count_even": (COUNT_SQL, count), "sum_even": (SUM_SQL, total)}
    cases = []
    for name in CASES:
        a, b = baseline.get(name), candidate.get(name)
        if not isinstance(a, dict) or not isinstance(b, dict):
            raise ValueError(f"missing {name} results")
        if a.get("verified") is not True or b.get("verified") is not True:
            raise ValueError(f"{name} must have verified reference output in both reports")
        if a.get("query") != b.get("query") or a.get("expected") != b.get("expected"):
            raise ValueError(f"{name} is not the same query and oracle")
        sql, oracle = expected[name]
        if (a.get("query") != sql or b.get("query") != sql
                or type(a.get("expected")) is not int
                or type(b.get("expected")) is not int
                or a["expected"] != oracle or b["expected"] != oracle):
            raise ValueError(f"{name}: incorrect physical SQL or independent oracle")
        for report, item in ((baseline, a), (candidate, b)):
            raw = item.get("samples_seconds")
            if raw is None:
                continue
            if (not isinstance(raw, list) or not raw
                    or any(type(value) not in (int, float)
                           or not math.isfinite(value) or value <= 0 for value in raw)):
                raise ValueError(f"{name}: invalid raw timing samples")
            if ("runs" in report and
                    (type(report["runs"]) is not int or len(raw) != report["runs"])):
                raise ValueError(f"{name}: sample count disagrees with measured runs")
            raw_median = statistics.median(raw)
            raw_p90 = sorted(raw)[math.ceil(0.9 * len(raw)) - 1]
            reported_median, reported_p90 = item.get("median_seconds"), item.get("p90_seconds")
            if (type(reported_median) not in (int, float)
                    or not math.isclose(reported_median, raw_median, rel_tol=1e-9)
                    or type(reported_p90) not in (int, float)
                    or not math.isclose(reported_p90, raw_p90, rel_tol=1e-9)):
                raise ValueError(f"{name}: timing summary differs from raw samples")
        if (a.get("samples_seconds") is None) != (b.get("samples_seconds") is None):
            raise ValueError(f"{name}: baseline and candidate timing samples differ")
        old, new = a.get("median_seconds"), b.get("median_seconds")
        if not all(type(x) in (int, float) and math.isfinite(x) and x > 0 for x in (old, new)):
            raise ValueError(f"{name} has invalid median timings")
        slowdown = 100 * (new / old - 1)
        old_p90, new_p90 = a.get("p90_seconds"), b.get("p90_seconds")
        tail_slowdown = None
        if old_p90 is not None or new_p90 is not None:
            if not all(type(x) in (int, float) and math.isfinite(x) and x >= median
                       for x, median in ((old_p90, old), (new_p90, new))):
                raise ValueError(f"{name}: invalid p90 benchmark timings")
            tail_slowdown = 100 * (new_p90 / old_p90 - 1)
        if max_tail_slowdown_percent is not None and tail_slowdown is None:
            raise ValueError(f"{name}: explicit tail budget requires p90 samples")
        tail_budget = (max_slowdown_percent if max_tail_slowdown_percent is None
                       else max_tail_slowdown_percent)
        within = slowdown <= max_slowdown_percent and (
            tail_slowdown is None or tail_slowdown <= tail_budget)
        cases.append({"name": name, "baseline_median_seconds": old,
                      "candidate_median_seconds": new, "slowdown_percent": slowdown,
                      "p90_slowdown_percent": tail_slowdown, "within_budget": within})
    return {"workload": "persisted-scan-regression-comparison",
            "rows": rows, "max_slowdown_percent": max_slowdown_percent,
            "max_tail_slowdown_percent": max_tail_slowdown_percent,
            "all_within_budget": all(case["within_budget"] for case in cases),
            "cases": cases, "shared_ci_runners_are_noisy": True}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--max-slowdown-percent", type=float, default=10)
    parser.add_argument("--max-tail-slowdown-percent", type=float)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = compare(json.loads(args.baseline.read_text(encoding="utf-8")),
                         json.loads(args.candidate.read_text(encoding="utf-8")),
                         max_slowdown_percent=args.max_slowdown_percent,
                         max_tail_slowdown_percent=args.max_tail_slowdown_percent)
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

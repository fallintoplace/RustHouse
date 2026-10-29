#!/usr/bin/env python3
"""Assess timing noise in a RustHouse TPC-H JSON benchmark report."""
import argparse
import json
import math
import statistics
from pathlib import Path


def assess_query(result, min_samples=5, max_cv_percent=10.0):
    if (type(min_samples) is not int or min_samples < 2
            or type(max_cv_percent) not in (int, float)
            or not math.isfinite(max_cv_percent) or max_cv_percent < 0):
        raise ValueError("invalid noise analysis thresholds")
    number = result["query"]
    if result.get("status") != "passed":
        return {"query": number, "state": "not_verified", "cv_percent": None,
                "mad_seconds": None, "samples": 0}
    samples = result.get("timings_seconds")
    if not isinstance(samples, list):
        return {"query": number, "state": "invalid_samples", "cv_percent": None,
                "mad_seconds": None, "samples": 0}
    if any(type(x) not in (int, float) or not math.isfinite(x) or x <= 0
           for x in samples):
        return {"query": number, "state": "invalid_samples", "cv_percent": None,
                "mad_seconds": None, "samples": len(samples)}
    if len(samples) < min_samples:
        return {"query": number, "state": "insufficient_samples", "cv_percent": None,
                "mad_seconds": None, "samples": len(samples)}
    mean = statistics.mean(samples)
    cv = 100 * statistics.pstdev(samples) / mean
    middle = statistics.median(samples)
    mad = statistics.median(abs(x - middle) for x in samples)
    return {"query": number, "state": "stable" if cv <= max_cv_percent else "noisy",
            "cv_percent": round(cv, 4), "mad_seconds": mad, "samples": len(samples)}


def assess_report(report, min_samples=5, max_cv_percent=10.0):
    if (not isinstance(report, dict) or not isinstance(report.get("results"), list)
            or not report["results"]):
        raise ValueError("invalid TPC-H report")
    seen = set()
    for item in report["results"]:
        if (not isinstance(item, dict) or type(item.get("query")) is not int
                or item["query"] < 1 or item["query"] in seen):
            raise ValueError("noise report needs distinct positive query IDs")
        seen.add(item["query"])
    return {
        "min_samples": min_samples, "max_cv_percent": max_cv_percent,
        "queries": [assess_query(item, min_samples, max_cv_percent)
                    for item in report["results"]],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--min-samples", type=int, default=5)
    parser.add_argument("--max-cv-percent", type=float, default=10.0)
    parser.add_argument("--gate", action="store_true",
                        help="Fail if any query is not verified, stable, or sufficiently sampled")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = assess_report(json.loads(args.input.read_text(encoding="utf-8")),
                               args.min_samples, args.max_cv_percent)
    except (ValueError, OSError, KeyError, TypeError) as error:
        parser.exit(1, f"noise analysis failed: {error}\n")
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return int(args.gate and any(q["state"] != "stable" for q in report["queries"]))


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Audit paired ClickHouse reports before making competitive performance claims.

This audits a reference-verified, fresh-CLI experiment, not an official
TPC-H benchmark and not an in-process engine-throughput comparison.
"""
import argparse
import json
import math
import re
import statistics
from pathlib import Path

HEX256 = re.compile(r"[0-9a-f]{64}\Z")


def finite_positive(value):
    return type(value) in (int, float) and math.isfinite(value) and value > 0


def accurate(observed, expected):
    return finite_positive(observed) and math.isclose(
        observed, expected, rel_tol=1e-9, abs_tol=0.0
    )


def summarize(report, min_queries=1, min_geomean=None, min_worst_p90=None):
    if type(min_queries) is not int or min_queries < 1:
        raise ValueError("min queries must be a positive integer")
    if min_geomean is not None and not finite_positive(min_geomean):
        raise ValueError("min geomean must be finite and positive")
    if min_worst_p90 is not None and not finite_positive(min_worst_p90):
        raise ValueError("min worst p90 must be finite and positive")
    if not isinstance(report, dict):
        raise ValueError("benchmark report must be an object")
    if report.get("workload") != "paired-clickhouse-cli-sql-latency":
        raise ValueError("expected a paired ClickHouse CLI benchmark")
    if report.get("official_tpch_score") is not False:
        raise ValueError("paired CLI results are not official TPC-H scores")
    if report.get("all_queries_verified") is not True:
        raise ValueError("all benchmark queries must be reference verified")
    if report.get("comparison") not in ("exact", "unordered"):
        raise ValueError("invalid result comparison mode")
    fingerprint = report.get("query_sha256")
    if not isinstance(fingerprint, str) or not HEX256.fullmatch(fingerprint):
        raise ValueError("missing or invalid query fingerprint")
    runs = report.get("runs")
    if type(runs) is not int or runs < 1:
        raise ValueError("invalid measured run count")
    results = report.get("results")
    if not isinstance(results, list) or len(results) < min_queries:
        raise ValueError("insufficient benchmark query coverage")

    seen = set()
    ratios = []
    tail_ratios = []
    for result in results:
        if not isinstance(result, dict) or result.get("status") != "passed":
            raise ValueError("a query is not verified")
        number = result.get("query")
        if type(number) is not int or not 1 <= number <= 22 or number in seen:
            raise ValueError("query IDs must be unique integers in 1..22")
        seen.add(number)
        reference = result.get("reference_sha256")
        if not isinstance(reference, str) or not HEX256.fullmatch(reference):
            raise ValueError(f"query {number}: invalid reference hash")
        samples = {}
        for engine in ("rusthouse", "clickhouse"):
            values = result.get(f"{engine}_samples_seconds")
            if (not isinstance(values, list) or len(values) != 2 * runs
                    or not all(finite_positive(value) for value in values)):
                raise ValueError(f"query {number}: invalid {engine} samples")
            median = statistics.median(values)
            p90 = sorted(values)[math.ceil(len(values) * 0.9) - 1]
            if not accurate(result.get(f"{engine}_median_seconds"), median):
                raise ValueError(f"query {number}: {engine} median does not match samples")
            if not accurate(result.get(f"{engine}_p90_seconds"), p90):
                raise ValueError(f"query {number}: {engine} p90 does not match samples")
            samples[engine] = (median, p90)
        ratio = samples["clickhouse"][0] / samples["rusthouse"][0]
        tail = samples["clickhouse"][1] / samples["rusthouse"][1]
        if not accurate(result.get("clickhouse_over_rusthouse"), ratio):
            raise ValueError(f"query {number}: incorrect median ratio")
        if not accurate(result.get("clickhouse_p90_over_rusthouse"), tail):
            raise ValueError(f"query {number}: incorrect tail ratio")
        ratios.append(ratio)
        tail_ratios.append(tail)

    geomean = math.exp(statistics.mean(math.log(value) for value in ratios))
    if not accurate(report.get("speedup_geomean_clickhouse_over_rusthouse"), geomean):
        raise ValueError("aggregate speedup is not supported by verified query ratios")
    return {
        "workload": "audited-paired-clickhouse-cli",
        "official_tpch_score": False,
        "reference_verified": True,
        "query_sha256": fingerprint,
        "audited_queries": len(results),
        "required_queries": min_queries,
        "geomean_clickhouse_over_rusthouse": geomean,
        "min_geomean": min_geomean,
        "min_worst_p90": min_worst_p90,
        "meets_speedup_target": ((min_geomean is None or geomean >= min_geomean)
                                 and (min_worst_p90 is None
                                      or min(tail_ratios) >= min_worst_p90)),
        "worst_p90_clickhouse_over_rusthouse": min(tail_ratios),
        "interpretation": "Ratios above 1 mean RustHouse was faster in this paired CLI scope",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--min-queries", type=int, default=1)
    parser.add_argument("--min-geomean", type=float)
    parser.add_argument("--min-worst-p90", type=float)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        audit = summarize(
            json.loads(args.input.read_text(encoding="utf-8")),
            min_queries=args.min_queries,
            min_geomean=args.min_geomean,
            min_worst_p90=args.min_worst_p90,
        )
    except (OSError, ValueError, TypeError, OverflowError) as error:
        parser.exit(1, f"paired ClickHouse audit failed: {error}\n")
    rendered = json.dumps(audit, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if audit["meets_speedup_target"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

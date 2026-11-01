#!/usr/bin/env python3
"""Assess timing noise in a RustHouse TPC-H JSON benchmark report."""
import argparse
import json
import math
import os
import stat
import statistics
import tempfile
from pathlib import Path

MAX_REPORT_BYTES = 2 * 1024 * 1024


def _unique_json_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate noise report JSON field: {key}")
        result[key] = value
    return result


def _reject_nonfinite_json(value):
    raise ValueError(f"nonfinite noise report JSON value: {value}")


def _finite_json_float(text):
    parsed = float(text)
    if not math.isfinite(parsed):
        raise ValueError("nonfinite noise report JSON value")
    return parsed


def load_report(path):
    path = Path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("report must be a regular, non-symlinked file")

    def identity(info):
        return (info.st_dev, info.st_ino, info.st_size,
                info.st_mtime_ns, info.st_ctime_ns)

    fd = os.open(path, os.O_RDONLY | getattr(os, "O_BINARY", 0)
                 | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(fd, "rb") as source:
        opened = os.fstat(source.fileno())
        if not stat.S_ISREG(opened.st_mode) or identity(opened) != identity(before):
            raise ValueError("noise report changed while opening")
        payload = source.read(MAX_REPORT_BYTES + 1)
        if identity(opened) != identity(os.fstat(source.fileno())):
            raise ValueError("noise report changed while reading")
    if identity(before) != identity(path.lstat()):
        raise ValueError("noise report changed while reading")
    if len(payload) > MAX_REPORT_BYTES:
        raise ValueError("benchmark report exceeds 2 MiB")
    return json.loads(payload, object_pairs_hook=_unique_json_fields,
                      parse_constant=_reject_nonfinite_json,
                      parse_float=_finite_json_float)


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
    cv = 100 * (statistics.pstdev(samples) / mean)
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
        report = assess_report(load_report(args.input),
                               args.min_samples, args.max_cv_percent)
    except (ValueError, OSError, KeyError, TypeError) as error:
        parser.exit(1, f"noise analysis failed: {error}\n")
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        if args.output.resolve() == args.input.resolve():
            parser.error("noise output must not replace its input")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        pending = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w", encoding="utf-8", dir=args.output.parent,
                prefix=f".{args.output.name}.", suffix=".tmp", delete=False,
            ) as stream:
                pending = Path(stream.name)
                stream.write(rendered)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(pending, args.output)
        finally:
            if pending is not None:
                pending.unlink(missing_ok=True)
    print(rendered, end="")
    return int(args.gate and any(q["state"] != "stable" for q in report["queries"]))


if __name__ == "__main__":
    raise SystemExit(main())

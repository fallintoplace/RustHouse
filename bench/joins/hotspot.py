"""Reference-verified join benchmark with repeated hot fact keys.

The build side has unique keys. A configurable percentage of fact rows use
key zero, stressing hash probe reuse and duplicate output generation.
This is a synthetic workload, not an official cross-engine benchmark.
"""
import argparse
from collections import Counter
import math
import tempfile
import time
from pathlib import Path

from bench.common import cli_sql, emit_json, latency_summary

QUERY = "SELECT f.id, d.label FROM facts f JOIN dimensions d ON f.id = d.id"


def fact_id(index, build_rows, hot_percent):
    if index % 100 < hot_percent or build_rows == 1:
        return 0
    return 1 + (index % (build_rows - 1))


def group_label(key):
    return f"group-{key % 10}"


def write_fixture(directory, probe_rows, build_rows, hot_percent):
    if type(probe_rows) is not int or not 1 <= probe_rows <= 131072:
        raise ValueError("probe rows must be in 1..131072")
    if type(build_rows) is not int or not 1 <= build_rows <= 32768:
        raise ValueError("build rows must be in 1..32768")
    if type(hot_percent) is not int or not 0 <= hot_percent <= 100:
        raise ValueError("hot percent must be 0..100")
    directory = Path(directory)
    if directory.is_symlink():
        raise ValueError("hotspot fixture root must not be a symlink")
    directory.mkdir(parents=True, exist_ok=True)
    for name in ("facts.tsv", "dimensions.tsv"):
        target = directory / name
        if target.is_symlink():
            raise ValueError(f"refusing symlinked hotspot fixture: {name}")
        if target.exists():
            raise FileExistsError(f"refusing to overwrite hotspot fixture: {name}")
    expected = Counter()
    with (directory / "facts.tsv").open("w", encoding="utf-8") as facts:
        facts.write("id\tamount\n")
        for index in range(probe_rows):
            key = fact_id(index, build_rows, hot_percent)
            facts.write(f"{key}\t{index % 100}\n")
            expected[(key, group_label(key))] += 1
    with (directory / "dimensions.tsv").open("w", encoding="utf-8") as dimensions:
        dimensions.write("id\tlabel\n")
        for key in range(build_rows):
            dimensions.write(f"{key}\t{group_label(key)}\n")
    return expected


def verify_rows(output, expected):
    if output and not output.endswith(b"\n"):
        raise ValueError("JOIN output is missing a final newline")
    received = Counter()
    for row in output.splitlines():
        fields = row.decode("utf-8").split("\t")
        if len(fields) != 2:
            raise ValueError("JOIN output has an invalid row")
        key = int(fields[0])
        if fields[0] != str(key):
            raise ValueError("JOIN output has a noncanonical key")
        received[(key, fields[1])] += 1
    if received != expected:
        raise ValueError("JOIN output differs from the independent row multiset")


def fixture_copy_sql(table, path):
    if table not in ("facts", "dimensions"):
        raise ValueError("unsupported fixture table")
    literal = str(path).replace("'", "''")
    return f"COPY {table} FROM '{literal}' WITH (FORMAT TSV, HEADER TRUE)"


def benchmark(binary, probe_rows=65536, build_rows=1024, hot_percent=90,
              runs=5, warmups=2, timeout=120):
    if (type(runs) is not int or type(warmups) is not int
            or runs < 1 or warmups < 0 or runs + warmups > 100
            or type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0):
        raise ValueError("invalid benchmark runs, warmups or timeout")
    with tempfile.TemporaryDirectory(prefix="rusthouse-join-hotspot-") as root:
        root = Path(root)
        expected = write_fixture(root, probe_rows, build_rows, hot_percent)
        database = root / "db"
        run = lambda sql: cli_sql(binary, database, sql, timeout)
        for sql in [
            "CREATE TABLE facts (id UInt64, amount UInt64)",
            "CREATE TABLE dimensions (id UInt64, label String)",
            fixture_copy_sql("facts", root / "facts.tsv"),
            fixture_copy_sql("dimensions", root / "dimensions.tsv"),
        ]:
            run(sql)
        samples = []
        for index in range(-warmups, runs):
            started = time.perf_counter()
            output = run(QUERY)
            seconds = time.perf_counter() - started
            verify_rows(output, expected)
            if index >= 0:
                samples.append(seconds)
    summary = latency_summary(samples)
    median = summary["median_seconds"]
    return {
        "workload": "skewed-hot-key-join",
        "official_benchmark": False,
        "probe_rows": probe_rows,
        "build_rows": build_rows,
        "hot_percent": hot_percent,
        "verified_output_rows": sum(expected.values()),
        "runs": runs,
        "warmups": warmups,
        "median_seconds": median,
        "p90_seconds": summary["p90_seconds"],
        "logical_probe_rows_per_second": probe_rows / median,
        "samples_seconds": samples,
        "timing_scope": "fresh CLI process with warm OS cache possible",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--probe-rows", type=int, default=65536)
    parser.add_argument("--build-rows", type=int, default=1024)
    parser.add_argument("--hot-percent", type=int, default=90)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = benchmark(args.binary, args.probe_rows, args.build_rows,
                           args.hot_percent, args.runs, args.warmups, args.timeout)
    except (OSError, RuntimeError, ValueError) as error:
        parser.exit(1, f"hot-key JOIN benchmark failed: {error}\n")
    emit_json(report, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Profile exact UInt32 DISTINCT across sparse, dense and mixed keyspaces.

This is a measured local RustHouse workload, not a ClickHouse speed claim.
Linux /proc sampling is used for peak RSS. The output is checked against a
separate Python exact-set oracle before accepting timing measurements.
"""
import argparse
import math
import statistics
import tempfile
from pathlib import Path

from bench.common import cli_sql, emit_json
from bench.scale.query_resource_profile import profile

MAX_ROWS = 200_000


def distinct_values(rows, layout):
    if type(rows) is not int or not 1 <= rows <= MAX_ROWS:
        raise ValueError("rows must be in 1..200000")
    if layout not in ("sparse", "dense", "mixed"):
        raise ValueError("layout must be sparse, dense or mixed")
    for index in range(rows):
        if layout == "sparse":
            yield ((index % 32768) << 16) | (index // 32768)
        elif layout == "dense":
            yield index % 65536
        else:
            yield ((index % 32768) << 16) | (index // 32768) if index % 2 == 0 else index % 65536


def write_tsv(path, rows, layout, duplicates):
    if (type(duplicates) is not int or not 1 <= duplicates <= 16
            or type(rows) is not int or not 1 <= rows <= MAX_ROWS
            or rows * duplicates > 1_000_000):
        raise ValueError("duplicates must be 1..16 and total rows <=1000000")
    if layout not in ("sparse", "dense", "mixed"):
        raise ValueError("layout must be sparse, dense or mixed")
    unique = set()
    with Path(path).open("w", encoding="ascii") as output:
        output.write("id\n")
        pending = []
        for value in distinct_values(rows, layout):
            unique.add(value)
            pending.extend([f"{value}\n"] * duplicates)
            if len(pending) >= 8192:
                output.write("".join(pending))
                pending.clear()
        if pending:
            output.write("".join(pending))
    return len(unique)


def benchmark(binary, rows=32768, layout="sparse", duplicates=2,
              runs=3, timeout=120, max_rss_mib=None, sample_ms=25):
    if type(runs) is not int or not 1 <= runs <= 100:
        raise ValueError("runs must be in 1..100")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    if (type(sample_ms) is not int or not 1 <= sample_ms <= 1000
            or (max_rss_mib is not None
                and (type(max_rss_mib) not in (int, float)
                     or not math.isfinite(max_rss_mib) or max_rss_mib <= 0))):
        raise ValueError("invalid process sampling budget")
    with tempfile.TemporaryDirectory(prefix="rusthouse-distinct32-memory-") as temp:
        root = Path(temp)
        fixture = root / "events.tsv"
        unique = write_tsv(fixture, rows, layout, duplicates)
        database = root / "db"
        cli_sql(binary, database, "CREATE TABLE events (id UInt32)", timeout)
        source = fixture.resolve().as_posix().replace("'", "''")
        cli_sql(binary, database,
                f"COPY events FROM '{source}' WITH (FORMAT TSV, HEADER TRUE)", timeout)
        query = "SELECT count(DISTINCT id) FROM events"
        expected = str(unique).encode()
        results = []
        for _ in range(runs):
            output = cli_sql(binary, database, query, timeout).strip()
            if output != expected:
                raise ValueError(f"wrong exact cardinality: {output!r}, expected {expected!r}")
            results.append(profile(
                binary, database, query, timeout=timeout,
                max_rss_mib=max_rss_mib, sample_ms=sample_ms,
            ))
    return {
        "workload": "exact-uint32-distinct-memory",
        "official_benchmark": False,
        "layout": layout,
        "rows": rows * duplicates,
        "expected_unique": unique,
        "runs": runs,
        "peak_rss_bytes_sampled_max": max(r["peak_rss_bytes_sampled"] for r in results),
        "median_seconds": statistics.median(r["seconds"] for r in results),
        "samples": results,
        "note": "Linux /proc sampled RSS; verify exact cardinality against Python set",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rows", type=int, default=32768)
    parser.add_argument("--layout", choices=("sparse", "dense", "mixed"), default="sparse")
    parser.add_argument("--duplicates", type=int, default=2)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--sample-ms", type=int, default=25)
    parser.add_argument("--max-rss-mib", type=float)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = benchmark(args.binary, args.rows, args.layout, args.duplicates,
                           args.runs, args.timeout, args.max_rss_mib, args.sample_ms)
    except (OSError, RuntimeError, ValueError) as error:
        parser.exit(1, f"UInt32 DISTINCT memory benchmark failed: {error}\n")
    emit_json(report, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

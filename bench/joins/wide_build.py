"""Compare wide and narrow dimension JOINs with verified output and RSS.

The same keys and source cardinality are loaded into two physical tables.
Only one table includes a large unreferenced String payload. This isolates
hash-build pruning without changing the relational result.
"""
import argparse
import math
import statistics
import tempfile
from pathlib import Path

from bench.common import cli_sql, emit_json
from bench.scale.query_resource_profile import profile


def write_dimensions(directory, rows, payload_bytes):
    if not 1 <= rows <= 32768:
        raise ValueError("build rows must be 1..32768")
    if not 1 <= payload_bytes <= 65536 or rows * payload_bytes > 128 * 1024 * 1024:
        raise ValueError("wide payload exceeds the 128 MiB fixture budget")
    directory = Path(directory)
    narrow = directory / "narrow.tsv"
    wide = directory / "wide.tsv"
    payload = "x" * payload_bytes
    with narrow.open("w", encoding="ascii") as small, wide.open("w", encoding="ascii") as large:
        small.write("id\n")
        large.write("id\tpayload\n")
        for key in range(rows):
            small.write(f"{key}\n")
            large.write(f"{key}\t{payload}\n")
    return narrow, wide


def benchmark(binary, fact_rows=65536, build_rows=8192, payload_bytes=4096,
              runs=3, timeout=120, max_rss_mib=None, sample_ms=25):
    if not 1 <= fact_rows <= 2_000_000 or fact_rows < build_rows:
        raise ValueError("fact rows must be >= build rows and <= 2 million")
    if runs < 1 or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("runs and timeout must be positive")
    with tempfile.TemporaryDirectory(prefix="rusthouse-wide-join-") as temp:
        root = Path(temp)
        narrow, wide = write_dimensions(root, build_rows, payload_bytes)
        database = root / "db"
        run = lambda sql: cli_sql(binary, database, sql, timeout)
        for sql in [
            "CREATE TABLE facts (id UInt64)",
            "CREATE TABLE narrow (id UInt64)",
            "CREATE TABLE wide (id UInt64, payload String)",
            f"INSERT INTO facts SELECT number FROM numbers({fact_rows})",
        ]:
            run(sql)
        for table, fixture in (("narrow", narrow), ("wide", wide)):
            path = fixture.resolve().as_posix().replace("'", "''")
            run(f"COPY {table} FROM '{path}' WITH (FORMAT TSV, HEADER TRUE)")

        summary = {}
        for table in ("narrow", "wide"):
            queries = {
                "count": f"SELECT count(*) FROM facts f JOIN {table} d ON f.id = d.id",
                "page": f"SELECT f.id FROM facts f JOIN {table} d ON f.id = d.id LIMIT 32",
            }
            for kind, query in queries.items():
                output = run(query)
                expected = (f"{build_rows}\n".encode() if kind == "count" else
                            "".join(f"{key}\n" for key in range(min(32, build_rows))).encode())
                if output != expected:
                    raise ValueError(f"{table} {kind} query result differs from exact oracle")
                samples = []
                for _ in range(runs):
                    samples.append(profile(
                        binary, database, query, timeout=timeout,
                        max_rss_mib=max_rss_mib, sample_ms=sample_ms,
                    ))
                summary[f"{table}_{kind}"] = {
                    "median_seconds": statistics.median(x["seconds"] for x in samples),
                    "peak_rss_bytes_sampled_max": max(x["peak_rss_bytes_sampled"] for x in samples),
                    "samples": samples,
                }
    return {
        "workload": "wide-dimension-hash-join",
        "official_benchmark": False,
        "fact_rows": fact_rows, "build_rows": build_rows,
        "unused_payload_bytes": payload_bytes, "verified": True,
        "runs": runs, "results": summary,
        "note": "Linux /proc sampled RSS, identical join keys, unreferenced String payload",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--fact-rows", type=int, default=65536)
    parser.add_argument("--build-rows", type=int, default=8192)
    parser.add_argument("--payload-bytes", type=int, default=4096)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--sample-ms", type=int, default=25)
    parser.add_argument("--max-rss-mib", type=float)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = benchmark(args.binary, args.fact_rows, args.build_rows,
                           args.payload_bytes, args.runs, args.timeout,
                           args.max_rss_mib, args.sample_ms)
    except (OSError, ValueError, RuntimeError) as error:
        parser.exit(1, f"wide dimension JOIN benchmark failed: {error}\n")
    emit_json(report, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

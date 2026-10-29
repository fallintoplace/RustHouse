"""Run bounded-output-memory correctness soaks against a physical RustHouse table.

Usage: python -m bench.scale.query_correctness_soak --binary target/release/rusthouse
       --database ./db --sql 'SELECT count(*) FROM events' --expected-file count.tsv
Every warmup and timed run is compared byte-for-byte by digest and length.
"""
import argparse
import hashlib
import json
import math
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def percentile(samples, percentile):
    if not samples or not 0 < percentile <= 100:
        raise ValueError("invalid sample count or percentile")
    return sorted(samples)[math.ceil(len(samples) * percentile / 100) - 1]


def digest_file(source):
    digest = hashlib.sha256()
    length = 0
    for data in iter(lambda: source.read(64 * 1024), b""):
        digest.update(data)
        length += len(data)
    return digest.hexdigest(), length


def run_sql(binary, database, statement, timeout):
    with tempfile.TemporaryFile() as sink:
        started = time.perf_counter()
        process = subprocess.run(
            [str(binary), "sql", statement, "--database", str(database)],
            stdout=sink, stderr=subprocess.PIPE, timeout=timeout, check=False,
        )
        elapsed = time.perf_counter() - started
        if process.returncode:
            raise RuntimeError(process.stderr.decode("utf-8", "replace")[-500:])
        sink.seek(0)
        digest, length = digest_file(sink)
        return digest, length, elapsed


def soak(binary, database, sql, expected_file, runs=20, warmups=2, timeout=120,
         execute=None):
    if (type(runs) is not int or type(warmups) is not int or runs < 1
            or warmups < 0 or runs + warmups > 1_000):
        raise ValueError("runs and warmups must total 1..1000 executions")
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    with Path(expected_file).open("rb") as source:
        reference, size = digest_file(source)
    execute = execute or (lambda: run_sql(binary, database, sql, timeout))
    samples = []
    for attempt in range(runs + warmups):
        actual, bytes_written, seconds = execute()
        if (actual, bytes_written) != (reference, size):
            raise ValueError(f"query output drift on iteration {attempt + 1}")
        if not math.isfinite(seconds) or seconds < 0:
            raise ValueError("invalid elapsed query time")
        if attempt >= warmups:
            samples.append(seconds)
    return {"verified": True, "runs": runs, "warmups": warmups, "output_sha256": reference,
            "output_bytes": size, "p50_seconds": statistics.median(samples),
            "p95_seconds": percentile(samples, 95), "max_seconds": max(samples)}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", required=True)
    p.add_argument("--database", required=True)
    p.add_argument("--sql", required=True)
    p.add_argument("--expected-file", type=Path, required=True)
    p.add_argument("--runs", type=int, default=20)
    p.add_argument("--warmups", type=int, default=2)
    p.add_argument("--timeout", type=float, default=120)
    a = p.parse_args(argv)
    try:
        report = soak(a.binary, a.database, a.sql, a.expected_file,
                      a.runs, a.warmups, a.timeout)
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        p.exit(1, f"query correctness soak failed: {error}\n")
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

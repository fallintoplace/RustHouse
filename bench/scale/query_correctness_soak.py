"""Run bounded-output-memory correctness soaks against a physical RustHouse table.

Usage: python -m bench.scale.query_correctness_soak --binary target/release/rusthouse
       --database ./db --sql 'SELECT count(*) FROM events' --expected-file count.tsv
Every warmup and timed run is compared byte-for-byte by digest and length.
"""
import argparse
import hashlib
import json
import math
import os
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def percentile(samples, percentile):
    if (not samples or type(percentile) not in (int, float)
            or not math.isfinite(percentile) or not 0 < percentile <= 100):
        raise ValueError("invalid sample count or percentile")
    return sorted(samples)[math.ceil(len(samples) * percentile / 100) - 1]


def digest_file(source):
    digest = hashlib.sha256()
    length = 0
    while True:
        data = source.read(64 * 1024)
        if not data:
            break
        digest.update(data)
        length += len(data)
    return digest.hexdigest(), length


def run_sql(binary, database, statement, timeout, max_output_bytes=100_000_000):
    extra = {}
    if os.name == "posix":
        import resource

        def limit_file_size():
            _, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
            size_limit = (max_output_bytes if hard == resource.RLIM_INFINITY
                          else min(max_output_bytes, hard))
            resource.setrlimit(resource.RLIMIT_FSIZE, (size_limit, hard))

        extra["preexec_fn"] = limit_file_size
    with tempfile.TemporaryFile() as sink:
        started = time.perf_counter()
        process = subprocess.run(
            [str(binary), "sql", statement, "--database", str(database)],
            stdout=sink, stderr=subprocess.PIPE, timeout=timeout, check=False, **extra,
        )
        elapsed = time.perf_counter() - started
        sink.seek(0, os.SEEK_END)
        if sink.tell() > max_output_bytes:
            raise ValueError("query output exceeds max_output_bytes")
        if process.returncode:
            raise RuntimeError(process.stderr.decode("utf-8", "replace")[-500:])
        sink.seek(0)
        digest, length = digest_file(sink)
        return digest, length, elapsed


def soak(binary, database, sql, expected_file, runs=20, warmups=2, timeout=120,
         execute=None, max_output_bytes=100_000_000):
    if (type(runs) is not int or type(warmups) is not int or runs < 1
            or warmups < 0 or runs + warmups > 1_000):
        raise ValueError("runs and warmups must total 1..1000 executions")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    if (type(max_output_bytes) is not int or not 1 <= max_output_bytes <= 1_000_000_000):
        raise ValueError("max_output_bytes must be in 1..1000000000")
    if not isinstance(sql, str) or not sql.strip() or len(sql.encode("utf-8")) > 65_536:
        raise ValueError("SQL must be nonempty and at most 64 KiB")
    with Path(expected_file).open("rb") as source:
        source.seek(0, os.SEEK_END)
        if source.tell() > max_output_bytes:
            raise ValueError("reference output exceeds max_output_bytes")
        source.seek(0)
        reference, size = digest_file(source)
    execute = execute or (lambda: run_sql(binary, database, sql, timeout, max_output_bytes))
    samples = []
    for attempt in range(runs + warmups):
        actual, bytes_written, seconds = execute()
        if (type(bytes_written) is not int or bytes_written < 0
                or bytes_written > max_output_bytes):
            raise ValueError("query output exceeds max_output_bytes")
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
    p.add_argument("--max-output-bytes", type=int, default=100_000_000)
    a = p.parse_args(argv)
    try:
        report = soak(a.binary, a.database, a.sql, a.expected_file,
                      a.runs, a.warmups, a.timeout,
                      max_output_bytes=a.max_output_bytes)
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        p.exit(1, f"query correctness soak failed: {error}\n")
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

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
import re
import stat
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

from .query_digest import _has_sql_separator


def percentile(samples, percentile):
    if (not samples or type(percentile) not in (int, float)
            or not math.isfinite(percentile) or not 0 < percentile <= 100
            or any(type(sample) not in (int, float)
                   or not math.isfinite(sample) or sample < 0
                   for sample in samples)):
        raise ValueError("invalid sample count or percentile")
    return sorted(samples)[math.ceil(len(samples) * percentile / 100) - 1]


MAX_SOAK_OUTPUT_BYTES = 100 * 1024 * 1024


def digest_file(source, *, max_bytes=None):
    if max_bytes is None:
        max_bytes = MAX_SOAK_OUTPUT_BYTES
    if type(max_bytes) is not int or max_bytes < 1:
        raise ValueError("max_bytes must be a positive integer")
    digest = hashlib.sha256()
    length = 0
    while True:
        # Consume at most one extra byte to detect an oversized stream.
        data = source.read(min(64 * 1024, max_bytes - length + 1))
        if not data:
            break
        length += len(data)
        if length > max_bytes:
            raise ValueError("soak output exceeds configured byte limit")
        digest.update(data)
    return digest.hexdigest(), length


def run_sql(binary, database, statement, timeout):
    with tempfile.TemporaryFile() as sink, tempfile.TemporaryFile() as errors:
        started = time.perf_counter()
        extra = {}
        if os.name == "posix":
            import resource
            def cap_spooled_output():
                _, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
                limit = (MAX_SOAK_OUTPUT_BYTES + 1 if hard == resource.RLIM_INFINITY
                         else min(MAX_SOAK_OUTPUT_BYTES + 1, hard))
                resource.setrlimit(resource.RLIMIT_FSIZE, (limit, hard))
            extra["preexec_fn"] = cap_spooled_output
        process = subprocess.run(
            [str(binary), "sql", statement, "--database", str(database)],
            stdout=sink, stderr=errors, timeout=timeout, check=False, **extra,
        )
        elapsed = time.perf_counter() - started
        if process.returncode:
            sink.seek(0, 2)
            if sink.tell() > MAX_SOAK_OUTPUT_BYTES:
                raise ValueError("soak output exceeds configured byte limit")
            errors.seek(0, 2)
            errors.seek(max(0, errors.tell() - 500))
            raise RuntimeError(errors.read(500).decode("utf-8", "replace"))
        sink.seek(0)
        digest, length = digest_file(sink)
        return digest, length, elapsed


def _file_state(meta):
    return (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns,
            meta.st_ctime_ns)


def read_reference(path):
    """Hash the same regular reference inode before and after reading it."""
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("expected output must be a regular non-symlinked file")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
    with os.fdopen(descriptor, "rb") as source:
        opened = os.fstat(source.fileno())
        if not stat.S_ISREG(opened.st_mode) or _file_state(before) != _file_state(opened):
            raise ValueError("expected output changed before opening")
        reference, size = digest_file(source)
        if _file_state(opened) != _file_state(os.fstat(source.fileno())):
            raise ValueError("expected output changed during hashing")
    if _file_state(before) != _file_state(path.lstat()):
        raise ValueError("expected output changed during hashing")
    return reference, size


def soak(binary, database, sql, expected_file, runs=20, warmups=2, timeout=120,
         execute=None):
    if (type(runs) is not int or type(warmups) is not int or runs < 1
            or warmups < 0 or runs + warmups > 1_000):
        raise ValueError("runs and warmups must total 1..1000 executions")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    if (not isinstance(sql, str) or not sql.strip()
            or len(sql.encode("utf-8")) > 65_536):
        raise ValueError("soak requires a bounded read-only SELECT")
    statement = sql.strip().removesuffix(";").rstrip()
    if not re.match(r"^SELECT\b", statement, re.IGNORECASE) or _has_sql_separator(statement):
        raise ValueError("soak requires one read-only SELECT")
    expected_file = Path(expected_file)
    reference, size = read_reference(expected_file)
    execute = execute or (lambda: run_sql(binary, database, sql, timeout))
    samples = []
    for attempt in range(runs + warmups):
        actual, bytes_written, seconds = execute()
        if (actual, bytes_written) != (reference, size):
            raise ValueError(f"query output drift on iteration {attempt + 1}")
        if type(seconds) not in (int, float) or seconds < 0:
            raise ValueError("invalid elapsed query time")
        try:
            finite = math.isfinite(seconds)
        except OverflowError:
            finite = False
        if not finite:
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

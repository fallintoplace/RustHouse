"""Reference-check random point queries against a known contiguous UInt64 ID domain.

Usage: python -m bench.scale.sparse_point_probes --binary target/release/rusthouse
       --database ./db --table events --rows 1000000000 --count 50
"""
import argparse
import json
import math
import random
import re
import statistics
import subprocess
import tempfile
import time


def choose_ids(start, rows, count, seed):
    if type(start) is not int or type(rows) is not int or start < 0 or rows < 1:
        raise ValueError("invalid row domain")
    if start + rows > 2**64 or type(count) is not int or not 1 <= count <= min(rows, 10_000):
        raise ValueError("invalid sample size or UInt64 overflow")
    if type(seed) is not int or not 0 <= seed <= (1 << 64) - 1:
        raise ValueError("seed must be UInt64")
    rng = random.Random(seed)
    chosen = set()
    sampled = []
    # Floyd's algorithm avoids retrying duplicate draws when the sample is dense.
    for upper in range(rows - count, rows):
        candidate = rng.randrange(upper + 1)
        selected = upper if candidate in chosen else candidate
        chosen.add(selected)
        sampled.append(start + selected)
    return sampled


def percentile(samples, percent):
    if (not samples or type(percent) not in (int, float)
            or not math.isfinite(percent) or not 0 < percent <= 100
            or any(type(sample) not in (int, float)
                   or not math.isfinite(sample) or sample < 0
                   for sample in samples)):
        raise ValueError("invalid percentile or latency samples")
    return percentile_sorted(sorted(samples), percent)


def percentile_sorted(ordered, percent):
    return ordered[math.ceil(percent * len(ordered) / 100) - 1]


def query(binary, database, statement, timeout):
    # A single point probe can only return a small result. Spool both streams.
    with tempfile.TemporaryFile(mode="w+b") as output, tempfile.TemporaryFile(mode="w+b") as errors:
        result = subprocess.run(
            [str(binary), "sql", statement, "--database", str(database),
             "--format", "tsv-no-header"],
            stdout=output, stderr=errors, timeout=timeout, check=False,
        )
        if result.returncode:
            errors.seek(0, 2)
            errors.seek(max(0, errors.tell() - 500))
            raise RuntimeError(errors.read(500).decode("utf-8", "replace"))
        output.seek(0)
        payload = output.read(129)
        if len(payload) > 128:
            raise ValueError("point probe SQL output exceeds 128 bytes")
        return payload

def probe(binary, database, table, start, rows, count=50, seed=0, timeout=30,
          execute=None, column="id"):
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", table):
        raise ValueError("unsafe table identifier")
    if not isinstance(column, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", column):
        raise ValueError("unsafe key column identifier")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    execute = execute or (lambda sql: query(binary, database, sql, timeout))
    sampled = choose_ids(start, rows, count, seed)
    times = []
    for value in sampled:
        statement = f"SELECT {column} FROM {table} WHERE {column} = {value} LIMIT 2"
        begun = time.perf_counter()
        actual = execute(statement)
        times.append(time.perf_counter() - begun)
        if actual != f"{value}\n".encode("ascii"):
            raise ValueError(f"point probe {value} returned {actual[:100]!r}")
    ordered = sorted(times)
    return {"verified": True, "rows_in_domain": rows, "samples": count,
            "column": column,
            "seed": seed, "p50_seconds": statistics.median(times),
            "p95_seconds": percentile_sorted(ordered, 95),
            "p99_seconds": percentile_sorted(ordered, 99),
            "max_seconds": max(times), "min_seconds": min(times)}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", required=True)
    p.add_argument("--database", required=True)
    p.add_argument("--table", required=True)
    p.add_argument("--column", default="id")
    p.add_argument("--start", type=int, default=0)
    p.add_argument("--rows", required=True, type=int)
    p.add_argument("--count", type=int, default=50)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--timeout", type=float, default=30)
    a = p.parse_args(argv)
    try:
        report = probe(a.binary, a.database, a.table, a.start,
                       a.rows, a.count, a.seed, a.timeout, column=a.column)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        p.exit(1, f"sparse point probe failed: {error}\n")
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

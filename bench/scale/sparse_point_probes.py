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
import time


def choose_ids(start, rows, count, seed):
    if type(start) is not int or type(rows) is not int or start < 0 or rows < 1:
        raise ValueError("invalid row domain")
    if start + rows > 2**64 or type(count) is not int or not 1 <= count <= min(rows, 10_000):
        raise ValueError("invalid sample size or UInt64 overflow")
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
    return sorted(samples)[math.ceil(percent * len(samples) / 100) - 1]


def query(binary, database, statement, timeout):
    result = subprocess.run(
        [str(binary), "sql", statement, "--database", str(database),
         "--format", "tsv-no-header"],
        capture_output=True, timeout=timeout, check=False,
    )
    if result.returncode:
        raise RuntimeError(result.stderr.decode("utf-8", "replace")[-500:])
    return result.stdout


def probe(binary, database, table, start, rows, count=50, seed=0, timeout=30,
          execute=None):
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", table):
        raise ValueError("unsafe table identifier")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    execute = execute or (lambda sql: query(binary, database, sql, timeout))
    sampled = choose_ids(start, rows, count, seed)
    times = []
    for value in sampled:
        statement = f"SELECT id FROM {table} WHERE id = {value} LIMIT 2"
        begun = time.perf_counter()
        actual = execute(statement)
        times.append(time.perf_counter() - begun)
        if actual != f"{value}\n".encode("ascii"):
            raise ValueError(f"point probe {value} returned {actual[:100]!r}")
    return {"verified": True, "rows_in_domain": rows, "samples": count,
            "seed": seed, "p50_seconds": statistics.median(times),
            "p95_seconds": percentile(times, 95),
            "p99_seconds": percentile(times, 99),
            "max_seconds": max(times), "min_seconds": min(times)}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", required=True)
    p.add_argument("--database", required=True)
    p.add_argument("--table", required=True)
    p.add_argument("--start", type=int, default=0)
    p.add_argument("--rows", required=True, type=int)
    p.add_argument("--count", type=int, default=50)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--timeout", type=float, default=30)
    a = p.parse_args(argv)
    try:
        report = probe(a.binary, a.database, a.table, a.start,
                       a.rows, a.count, a.seed, a.timeout)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        p.exit(1, f"sparse point probe failed: {error}\n")
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

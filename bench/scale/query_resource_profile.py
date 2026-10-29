"""Measure a RustHouse SQL subprocess on Linux using /proc (not a CPU profiler).

Usage: python -m bench.scale.query_resource_profile --binary target/release/rusthouse
       --database ./db --sql 'SELECT count(*) FROM events' --max-rss-mib 2048
"""
import argparse
import json
import math
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def parse_rss(status):
    peak = 0
    for line in status.splitlines():
        if line.startswith(("VmRSS:", "VmHWM:")):
            tokens = line.split()
            if len(tokens) == 3 and tokens[2] == "kB":
                peak = max(peak, int(tokens[1]) * 1024)
    return peak


def parse_io(payload):
    fields = {}
    for line in payload.splitlines():
        if ": " in line:
            key, value = line.split(": ", 1)
            fields[key] = int(value)
    return {key: fields.get(key, 0) for key in ("read_bytes", "write_bytes")}


def profile(binary, database, sql, timeout=120, max_rss_mib=None, sample_ms=25):
    if sys.platform != "linux":
        raise RuntimeError("process resource sampling requires Linux /proc")
    if not math.isfinite(timeout) or timeout <= 0 or not 1 <= sample_ms <= 1000:
        raise ValueError("invalid timeout or sample interval")
    if max_rss_mib is not None and (not math.isfinite(max_rss_mib) or max_rss_mib <= 0):
        raise ValueError("max RSS budget must be positive")
    started = time.perf_counter()
    stderr_file = tempfile.TemporaryFile(mode="w+b")
    try:
        process = subprocess.Popen(
            [str(binary), "sql", sql, "--database", str(database)],
            stdout=subprocess.DEVNULL, stderr=stderr_file,
        )
    except BaseException:
        stderr_file.close()
        raise
    peak = 0
    disk = {"read_bytes": 0, "write_bytes": 0}
    failed_budget = False
    timed_out = False
    try:
        proc = Path("/proc") / str(process.pid)
        while process.poll() is None:
            try:
                peak = max(peak, parse_rss((proc / "status").read_text()))
                disk = parse_io((proc / "io").read_text())
            except (OSError, ValueError):
                pass  # The process may exit between poll() and the /proc reads.
            if max_rss_mib is not None and peak > max_rss_mib * 1024 * 1024:
                failed_budget = True
                process.kill()
                break
            if time.perf_counter() - started > timeout:
                timed_out = True
                process.kill()
                break
            time.sleep(sample_ms / 1000)
        process.wait(timeout=5)
        stderr_file.seek(0, os.SEEK_END)
        stderr_file.seek(max(0, stderr_file.tell() - 500))
        stderr = stderr_file.read()
    except BaseException:
        process.kill()
        process.wait()
        raise
    finally:
        stderr_file.close()
    elapsed = time.perf_counter() - started
    if failed_budget:
        raise RuntimeError(f"query exceeded sampled RSS budget: {peak} bytes")
    if timed_out:
        raise RuntimeError(f"query exceeded timeout of {timeout}s")
    if process.returncode:
        raise RuntimeError(f"query exited {process.returncode}: {stderr.decode('utf-8', 'replace')[-500:]}")
    return {"seconds": elapsed, "peak_rss_bytes_sampled": peak, **disk,
            "sample_interval_ms": sample_ms,
            "note": "VmHWM gives the kernel RSS high-water mark when present; /proc I/O excludes page-cache hits."}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", required=True, type=Path)
    p.add_argument("--database", required=True, type=Path)
    p.add_argument("--sql", required=True)
    p.add_argument("--timeout", type=float, default=120)
    p.add_argument("--max-rss-mib", type=float)
    p.add_argument("--sample-ms", type=int, default=25)
    a = p.parse_args(argv)
    try:
        report = profile(a.binary, a.database, a.sql, a.timeout,
                         a.max_rss_mib, a.sample_ms)
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        p.exit(1, f"SQL resource profile failed: {error}\n")
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

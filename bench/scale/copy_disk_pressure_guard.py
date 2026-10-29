"""Watch available disk space while importing a large TSV shard.

Usage: python -m bench.scale.copy_disk_pressure_guard --binary target/release/rusthouse
       --database ./db --table events --source /data/part.tsv
       --watch-dir /data --min-free-gib 20
An aborted COPY may leave staging files; run rusthouse doctor before retrying.
"""
import argparse
import json
import math
import os
import re
import shutil
import subprocess
import tempfile
import time
from pathlib import Path


def sql_for(table, source):
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", table):
        raise ValueError("unsafe table identifier")
    path = str(Path(source).resolve())
    if len(path.encode("utf-8")) > 32768:
        raise ValueError("source path is too long")
    return f"COPY {table} FROM '{path.replace(chr(39), chr(39)*2)}' WITH (FORMAT TSV)"


def same_device(database, watch_dir):
    return os.stat(database).st_dev == os.stat(watch_dir).st_dev


def guard(binary, database, table, source, watch_dir, min_free_gib=5,
          timeout=3600, poll_seconds=0.25):
    if (type(min_free_gib) not in (int, float)
            or not math.isfinite(min_free_gib) or min_free_gib <= 0):
        raise ValueError("min-free-gib must be positive")
    if (type(timeout) not in (int, float)
            or not math.isfinite(timeout) or timeout <= 0):
        raise ValueError("timeout must be positive")
    if (type(poll_seconds) not in (int, float)
            or not math.isfinite(poll_seconds) or not 0.05 <= poll_seconds <= 10):
        raise ValueError("poll-seconds must be in 0.05..10")
    reserve_float = min_free_gib * (1024**3)
    if not math.isfinite(reserve_float) or not 1 <= reserve_float <= (1 << 63) - 1:
        raise ValueError("min-free-gib must correspond to 1..Int64 bytes")
    if not Path(source).is_file():
        raise ValueError("source TSV is missing")
    if not Path(watch_dir).is_dir():
        raise ValueError("watch directory is missing")
    if Path(database).exists() and not same_device(database, watch_dir):
        raise ValueError("watch directory is not on the database filesystem")
    reserve = math.ceil(reserve_float)
    free_start = shutil.disk_usage(watch_dir).free
    if free_start < reserve:
        raise RuntimeError("insufficient free disk space before COPY")
    command = [str(binary), "sql", sql_for(table, source),
               "--database", str(database)]
    started = time.perf_counter()
    stderr_file = tempfile.TemporaryFile(mode="w+b")
    try:
        process = subprocess.Popen(command, stdout=subprocess.DEVNULL,
                                   stderr=stderr_file)
    except BaseException:
        stderr_file.close()
        raise
    minimum = free_start
    timed_out = False
    pressure = False
    try:
        while process.poll() is None:
            available = shutil.disk_usage(watch_dir).free
            minimum = min(minimum, available)
            if available < reserve:
                pressure = True
                process.kill()
                break
            if time.perf_counter() - started > timeout:
                timed_out = True
                process.kill()
                break
            time.sleep(poll_seconds)
        process.wait(timeout=10)
        stderr_file.seek(0, os.SEEK_END)
        stderr_file.seek(max(0, stderr_file.tell() - 500))
        stderr = stderr_file.read()
    except BaseException:
        process.kill()
        process.wait()
        raise
    finally:
        stderr_file.close()
    minimum = min(minimum, shutil.disk_usage(watch_dir).free)
    if pressure or minimum < reserve:
        raise RuntimeError("COPY stopped: disk reserve exhausted")
    if timed_out:
        raise RuntimeError("COPY stopped: timeout exceeded")
    if process.returncode:
        raise RuntimeError(f"COPY failed: {stderr.decode('utf-8', 'replace')[-500:]}")
    return {"copied": True, "seconds": time.perf_counter() - started,
            "free_bytes_before": free_start, "lowest_free_bytes_observed": minimum,
            "reserve_bytes": reserve, "note": "Sampling may miss brief disk-space dips."}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", required=True, type=Path)
    p.add_argument("--database", required=True, type=Path)
    p.add_argument("--table", required=True)
    p.add_argument("--source", required=True, type=Path)
    p.add_argument("--watch-dir", required=True, type=Path)
    p.add_argument("--min-free-gib", type=float, default=5)
    p.add_argument("--timeout", type=float, default=3600)
    p.add_argument("--poll-seconds", type=float, default=0.25)
    a = p.parse_args(argv)
    try:
        report = guard(a.binary, a.database, a.table, a.source, a.watch_dir,
                       a.min_free_gib, a.timeout, a.poll_seconds)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        p.exit(1, f"guarded COPY failed: {error}\n")
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Run RustHouse's offline doctor and print a bounded JSON report."""
import argparse
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def report(returncode, stdout, stderr, elapsed, full_hashes):
    return {
        "schema_version": 1,
        "ok": returncode == 0,
        "exit_code": returncode,
        "elapsed_seconds": round(elapsed, 3),
        "full_hashes": full_hashes,
        "stdout": stdout[:32768],
        "stderr": stderr[:32768],
        "truncated": len(stdout) > 32768 or len(stderr) > 32768,
    }


def run(binary, database, full_hashes=True, timeout=600):
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or not 0 < timeout <= 3600:
        raise ValueError("timeout must be finite and in (0, 3600] seconds")
    command = [str(binary), "doctor", "--database", str(database)]
    if full_hashes:
        command.append("--full-hashes")
    start = time.monotonic()
    try:
        with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
            result = subprocess.run(command, stdout=output, stderr=errors,
                                    timeout=timeout, check=False)
            output.seek(0)
            errors.seek(0)
            out = output.read(32_769)
            err = errors.read(32_769)
        result_report = report(result.returncode, out.decode("utf-8", "replace"),
                               err.decode("utf-8", "replace"),
                               time.monotonic() - start, full_hashes)
        if len(out) > 32_768 or len(err) > 32_768:
            result_report["truncated"] = True
        return result_report
    except subprocess.TimeoutExpired:
        return report(124, "", "RustHouse doctor timed out",
                      time.monotonic() - start, full_hashes)
    except OSError as exc:
        return report(127, "", str(exc),
                      time.monotonic() - start, full_hashes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/rusthouse"))
    parser.add_argument("--offline-confirmed", action="store_true")
    parser.add_argument("--timeout", type=float, default=600,
                        help="Maximum doctor runtime in seconds (up to 3600)")
    parser.add_argument("--quick", action="store_true",
                        help="Skip the slower stored-byte full-hash validation")
    args = parser.parse_args()
    if not args.offline_confirmed:
        parser.error("stop all writers and readers, then pass --offline-confirmed")
    if not 0 < args.timeout <= 3600 or not math.isfinite(args.timeout):
        parser.error("timeout must be finite and in (0, 3600] seconds")
    result = run(args.binary, args.database, not args.quick, timeout=args.timeout)
    print(json.dumps(result, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

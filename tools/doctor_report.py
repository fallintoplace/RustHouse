#!/usr/bin/env python3
"""Run RustHouse's offline doctor and print a bounded JSON report."""
import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


# Child stdout and stderr are spooled to regular files. Capping the final
# JSON alone would allow a noisy or compromised doctor to exhaust the disk.
MAX_CHILD_OUTPUT_BYTES = 8 * 1024 * 1024


def _limit_child_output():
    import resource

    soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
    cap = min(soft, MAX_CHILD_OUTPUT_BYTES) if soft != resource.RLIM_INFINITY else MAX_CHILD_OUTPUT_BYTES
    if hard != resource.RLIM_INFINITY:
        cap = min(cap, hard)
    resource.setrlimit(resource.RLIMIT_FSIZE, (cap, hard))


def _bounded_text(value):
    payload = value.encode("utf-8")
    if len(payload) <= 32_768:
        return value, False
    return payload[:32_768].decode("utf-8", "ignore"), True


def report(returncode, stdout, stderr, elapsed, full_hashes):
    stdout, out_truncated = _bounded_text(stdout)
    stderr, err_truncated = _bounded_text(stderr)
    return {
        "schema_version": 1,
        "ok": returncode == 0,
        "exit_code": returncode,
        "elapsed_seconds": round(elapsed, 3),
        "full_hashes": full_hashes,
        "stdout": stdout,
        "stderr": stderr,
        "truncated": out_truncated or err_truncated,
    }


def run(binary, database, full_hashes=True, timeout=600):
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("doctor timeout must be positive and finite")
    command = [str(binary), "doctor", "--database", str(database)]
    if full_hashes:
        command.append("--full-hashes")
    start = time.monotonic()
    try:
        with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
            result = subprocess.run(command, stdout=output, stderr=errors,
                                    timeout=timeout, check=False,
                                    preexec_fn=_limit_child_output if os.name == "posix" else None)
            output.seek(0)
            out = output.read(32_769)
            errors.seek(0, 2)
            error_bytes = errors.tell()
            if result.returncode:
                # Surface the final fatal message rather than early progress lines.
                errors.seek(max(0, error_bytes - 32_768))
            else:
                errors.seek(0)
            err = errors.read(32_769)
        result_report = report(result.returncode, out.decode("utf-8", "replace"),
                               err.decode("utf-8", "replace"),
                               time.monotonic() - start, full_hashes)
        if len(out) > 32_768 or error_bytes > 32_768:
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
                        help="Maximum seconds for offline doctor verification")
    parser.add_argument("--quick", action="store_true",
                        help="Skip the slower stored-byte full-hash validation")
    args = parser.parse_args()
    if not args.offline_confirmed:
        parser.error("stop all writers and readers, then pass --offline-confirmed")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be positive and finite")
    result = run(args.binary, args.database, not args.quick, timeout=args.timeout)
    print(json.dumps(result, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

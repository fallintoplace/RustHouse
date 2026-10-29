#!/usr/bin/env python3
"""Run RustHouse's offline doctor and print a bounded JSON report."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time


OUTPUT_PREVIEW_CHARS = 32_768


def report(returncode, stdout, stderr, elapsed, full_hashes):
    return {
        "schema_version": 1,
        "ok": returncode == 0,
        "exit_code": returncode,
        "elapsed_seconds": round(elapsed, 3),
        "full_hashes": full_hashes,
        "stdout": stdout[:OUTPUT_PREVIEW_CHARS],
        "stderr": stderr[:OUTPUT_PREVIEW_CHARS],
        "truncated": len(stdout) > OUTPUT_PREVIEW_CHARS or len(stderr) > OUTPUT_PREVIEW_CHARS,
    }


def run(binary, database, full_hashes=True, timeout=600):
    command = [str(binary), "doctor", "--database", str(database)]
    if full_hashes:
        command.append("--full-hashes")
    start = time.monotonic()
    try:
        result = subprocess.run(command, capture_output=True, text=True,
                                timeout=timeout, check=False)
        return report(result.returncode, result.stdout, result.stderr,
                      time.monotonic() - start, full_hashes)
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
    parser.add_argument("--quick", action="store_true",
                        help="Skip the slower stored-byte full-hash validation")
    args = parser.parse_args()
    if not args.offline_confirmed:
        parser.error("stop all writers and readers, then pass --offline-confirmed")
    result = run(args.binary, args.database, not args.quick)
    print(json.dumps(result, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

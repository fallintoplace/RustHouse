#!/usr/bin/env python3
"""Local persisted-data product acceptance, not a power-loss durability test."""
import argparse
from pathlib import Path
import subprocess
import sys
import tempfile


def check_rows(output):
    if output != "id\towner\tamount\n1\tAda\t20\n2\tGrace\t30\n":
        raise AssertionError("unexpected persisted rows or row order")


def invoke(binary, *args):
    try:
        with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
            completed = subprocess.run(
                [str(binary), *args], stdout=output, stderr=errors,
                check=False, timeout=120,
            )
            errors.seek(0)
            error = errors.read(2000).decode("utf-8", "replace")
            if completed.returncode:
                raise RuntimeError("rusthouse failed: " + error)
            output.seek(0)
            data = output.read(65_537)
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError("rusthouse acceptance command timed out") from exc
    if len(data) > 65_536:
        raise RuntimeError("rusthouse acceptance output exceeds 64 KiB")
    return data.decode("utf-8", "replace")


def accept(binary):
    with tempfile.TemporaryDirectory(prefix="rusthouse-accept-") as db:
        invoke(binary, "sql", "CREATE TABLE trial (id UInt64, owner String, amount UInt64)", "--database", db)
        invoke(binary, "sql", "INSERT INTO trial VALUES (1, 'Ada', 20), (2, 'Grace', 30)", "--database", db)
        check_rows(invoke(binary, "sql", "SELECT id, owner, amount FROM trial ORDER BY id", "--database", db))
        invoke(binary, "doctor", "--database", db)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/rusthouse"))
    args = parser.parse_args()
    try:
        accept(args.binary.resolve())
    except (OSError, RuntimeError, AssertionError) as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    print("PASS: persisted rows and offline doctor")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

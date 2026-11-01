#!/usr/bin/env python3
"""Import bounded trial CSV data into a NEW local table."""
import argparse
import csv
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile

FIELDS = ["id", "region", "amount"]


def validate(path):
    if not path.is_file() or path.is_symlink():
        raise ValueError("CSV must be a regular file, not a symlink")
    if path.stat().st_size > 8 * 1024 * 1024:
        raise ValueError("CSV exceeds 8 MiB")
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames != FIELDS:
            raise ValueError("expected id,region,amount headers")
        count = 0
        for row in reader:
            count += 1
            if count > 50000 or None in row:
                raise ValueError("CSV has too many rows or fields")
            for key in ("id", "amount"):
                value = row[key]
                if not value or not value.isascii() or not value.isdigit():
                    raise ValueError("UInt64 fields must contain decimal digits")
                if int(value) >= 2**64:
                    raise ValueError("UInt64 field overflows")
        return count


def sql_path(path):
    return "'" + str(path.resolve()).replace("'", "''") + "'"


def _freeze_csv_source(source, frozen):
    """Copy a bounded, stable source into a private directory before validating."""
    source = Path(source)
    before = source.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("CSV must be a regular file, not a symlink")
    if before.st_size > 8 * 1024 * 1024:
        raise ValueError("CSV exceeds 8 MiB")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(source, flags)
    with os.fdopen(fd, "rb") as stream:
        opened = os.fstat(stream.fileno())
        if (not stat.S_ISREG(opened.st_mode)
                or (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino)):
            raise ValueError("CSV source changed while opening")
        content = stream.read(8 * 1024 * 1024 + 1)
        finished = os.fstat(stream.fileno())
        identity = lambda state: (state.st_dev, state.st_ino, state.st_size,
                                  state.st_mtime_ns, state.st_ctime_ns)
        if identity(opened) != identity(finished):
            raise ValueError("CSV source changed during copy")
    if identity(before) != identity(source.lstat()):
        raise ValueError("CSV source changed during copy")
    if len(content) > 8 * 1024 * 1024:
        raise ValueError("CSV exceeds 8 MiB")
    frozen.write_bytes(content)


def import_csv(binary, database, path, table, runner=subprocess.run):
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]{0,127}", table):
        raise ValueError("invalid SQL table identifier")
    # Validate and import the *same* bytes. The original path can otherwise
    # be replaced between the CSV check and the COPY subprocess.
    with tempfile.TemporaryDirectory(prefix="rusthouse-csv-import-") as directory:
        frozen = Path(directory) / "input.csv"
        _freeze_csv_source(path, frozen)
        count = validate(frozen)
        def execute(sql):
            runner([str(binary), "sql", sql, "--database", str(database)], check=True)
        execute("CREATE TABLE " + table + " (id UInt64, region String, amount UInt64)")
        execute("COPY " + table + " FROM " + sql_path(frozen) + " WITH (FORMAT CSV, HEADER TRUE)")
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("csv_file", type=Path)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--binary", default="target/debug/rusthouse")
    parser.add_argument("--table", default="trial_purchases")
    args = parser.parse_args()
    try:
        count = import_csv(args.binary, args.database, args.csv_file, args.table)
    except (ValueError, csv.Error, OSError, subprocess.CalledProcessError) as exc:
        print("Import failed: " + str(exc), file=sys.stderr)
        return 1
    print("Imported " + str(count) + " rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

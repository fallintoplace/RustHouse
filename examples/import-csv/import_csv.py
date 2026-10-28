#!/usr/bin/env python3
"""Import bounded trial CSV data into a NEW local table."""
import argparse
import csv
from pathlib import Path
import re
import subprocess
import sys

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


def import_csv(binary, database, path, table, runner=subprocess.run):
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]{0,127}", table):
        raise ValueError("invalid SQL table identifier")
    count = validate(path)
    def execute(sql):
        runner([str(binary), "sql", sql, "--database", str(database)], check=True)
    execute("CREATE TABLE " + table + " (id UInt64, region String, amount UInt64)")
    execute("COPY " + table + " FROM " + sql_path(path) + " WITH (FORMAT CSV, HEADER TRUE)")
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

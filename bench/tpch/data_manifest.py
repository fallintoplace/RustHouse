#!/usr/bin/env python3
"""Fingerprint converted dbgen TSV for reproducible cross-engine comparison."""
import argparse
import hashlib
import json
import os
import stat
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import file_change_time_ns
from schema import TABLES, names, explain_scaling

MAX_RECORD_BYTES = 8 * 1024 * 1024


def inspect_table(path, table):
    path = Path(path)
    try:
        before = path.lstat()
    except FileNotFoundError as error:
        raise ValueError(f"{table}: converted TSV does not exist: {path}") from error
    if not stat.S_ISREG(before.st_mode):
        raise ValueError(f"{table}: converted TSV must be a regular, non-symlinked file: {path}")
    expected = "\t".join(names(table)).encode("utf-8")
    digest = hashlib.sha256()
    rows = 0
    expected_columns = len(TABLES[table])
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(path, flags)
    with os.fdopen(fd, "rb") as source:
        opened = os.fstat(source.fileno())
        if (not stat.S_ISREG(opened.st_mode)
                or (before.st_dev, before.st_ino, before.st_size,
                    before.st_mtime_ns, file_change_time_ns(before)) !=
                   (opened.st_dev, opened.st_ino, opened.st_size,
                    opened.st_mtime_ns, file_change_time_ns(opened))):
            raise ValueError(f"{table}: TSV source changed before opening")
        header = source.readline(len(expected) + 3)
        if header not in (expected + b"\n", expected + b"\r\n"):
            raise ValueError(f"{table}: unexpected TSV header")
        digest.update(header)
        while record := source.readline(MAX_RECORD_BYTES + 1):
            if len(record) > MAX_RECORD_BYTES:
                raise ValueError(f"{table}: oversized TSV record")
            if record in (b"\n", b"\r\n", b""):
                raise ValueError(f"{table}: blank record")
            if not record.endswith(b"\n") or record.count(b"\t") != expected_columns - 1:
                raise ValueError(f"{table}: malformed or truncated TSV record")
            digest.update(record)
            rows += 1
        finished = os.fstat(source.fileno())
        if (opened.st_size, opened.st_mtime_ns, file_change_time_ns(opened)) != (
                finished.st_size, finished.st_mtime_ns, file_change_time_ns(finished)):
            raise ValueError(f"{table}: TSV source changed during hashing")
    after = path.lstat()
    if (before.st_dev, before.st_ino, before.st_size,
            before.st_mtime_ns, file_change_time_ns(before)) != (
            after.st_dev, after.st_ino, after.st_size,
            after.st_mtime_ns, file_change_time_ns(after)):
        raise ValueError(f"{table}: TSV source changed while hashing")
    if rows == 0:
        raise ValueError(f"{table}: no data rows")
    return {
        "name": table,
        "rows": rows,
        "bytes": before.st_size,
        "sha256": digest.hexdigest(),
        "columns": [{"name": name, "type": kind, "scale": scale}
                    for name, kind, scale in TABLES[table]],
    }


def build_manifest(input_dir, selected=None):
    selected = tuple(TABLES if selected is None else selected)
    if not selected or len(set(selected)) != len(selected):
        raise ValueError("table selection must be nonempty and unique")
    if any(table not in TABLES for table in selected):
        raise ValueError("unknown dbgen table")
    return {
        "format": "rusthouse-tpch-fixed-point-tsv-v1",
        "official_tpch_score": False,
        "scaling": explain_scaling(),
        "tables": [inspect_table(Path(input_dir) / (table + ".tsv"), table)
                   for table in selected],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--table", action="append", choices=tuple(TABLES))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = build_manifest(args.input_dir, args.table)
    except (OSError, ValueError) as error:
        parser.exit(1, f"manifest failed: {error}\n")
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

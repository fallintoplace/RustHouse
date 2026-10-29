#!/usr/bin/env python3
"""Fingerprint converted dbgen TSV for reproducible cross-engine comparison."""
import argparse
import hashlib
import json
from pathlib import Path

from schema import TABLES, names, explain_scaling

MAX_RECORD_BYTES = 8 * 1024 * 1024


def inspect_table(path, table):
    path = Path(path)
    if not path.is_file():
        raise ValueError(f"{table}: converted TSV does not exist: {path}")
    expected = "\t".join(names(table)).encode("utf-8")
    digest = hashlib.sha256()
    rows = 0
    expected_columns = len(TABLES[table])
    with path.open("rb") as source:
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
    if rows == 0:
        raise ValueError(f"{table}: no data rows")
    return {
        "name": table,
        "rows": rows,
        "bytes": path.stat().st_size,
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

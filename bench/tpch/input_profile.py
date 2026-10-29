#!/usr/bin/env python3
"""Fingerprint converted TPC-H data and record exact workload input sizes."""
import argparse
import hashlib
import json
from pathlib import Path
from schema import TABLES, names


def matches_q6(parts, indexes):
    date = parts[indexes["l_shipdate"]]
    return (
        "1994-01-01" <= date < "1995-01-01"
        and 500 <= int(parts[indexes["l_discount"]]) <= 700
        and int(parts[indexes["l_quantity"]]) < 2400
    )


def profile_table(path, table):
    if table not in TABLES:
        raise ValueError(f"unknown table: {table}")
    digest = hashlib.sha256()
    rows, bytes_read, q6_matches = 0, 0, 0
    min_shipdate = max_shipdate = None
    with Path(path).open("rb") as stream:
        header = stream.readline()
        digest.update(header)
        bytes_read += len(header)
        if header.decode("utf-8").rstrip("\r\n").split("\t") != names(table):
            raise ValueError(f"{table}: TSV header mismatch")
        indexes = {name: position for position, name in enumerate(names(table))}
        for line in stream:
            if not line.endswith(b"\n"):
                raise ValueError(f"{table}: truncated TSV data record")
            bytes_read += len(line)
            digest.update(line)
            parts = line.decode("utf-8").rstrip("\r\n").split("\t")
            if len(parts) != len(indexes):
                raise ValueError(f"{table}: row {rows + 1} has wrong column count")
            rows += 1
            if table == "lineitem":
                date = parts[indexes["l_shipdate"]]
                min_shipdate = date if min_shipdate is None else min(date, min_shipdate)
                max_shipdate = date if max_shipdate is None else max(date, max_shipdate)
                if matches_q6(parts, indexes):
                    q6_matches += 1
    result = {"table": table, "rows": rows, "size_bytes": bytes_read,
              "sha256": digest.hexdigest()}
    if table == "lineitem":
        result.update({"first_shipdate": min_shipdate, "last_shipdate": max_shipdate,
                       "q6_predicate_matches": q6_matches,
                       "q6_selectivity_percent": round(100 * q6_matches / rows, 4) if rows else 0})
    return result


def profile_directory(directory, selected=None):
    selected = tuple(TABLES if selected is None else selected)
    if not selected or len(set(selected)) != len(selected) or any(
        table not in TABLES for table in selected
    ):
        raise ValueError("select a nonempty unique set of known TPC-H tables")
    profiles = [profile_table(Path(directory) / (table + ".tsv"), table)
                for table in selected]
    return {"workload": "tpch-input-profile", "certified": False,
            "total_rows": sum(item["rows"] for item in profiles),
            "total_bytes": sum(item["size_bytes"] for item in profiles),
            "tables": profiles}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--table", action="append", choices=tuple(TABLES))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = profile_directory(args.input_dir, args.table)
    except (OSError, ValueError, UnicodeError) as error:
        parser.exit(1, f"TPC-H profile failed: {error}\n")
    content = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(content, encoding="utf-8")
    print(content, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Verify the eight SF1 dbgen table cardinalities before full TPC-H queries."""
import argparse
import json
import math
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql

SF1_ROWS = {
    "region": 5,
    "nation": 25,
    "supplier": 10_000,
    "customer": 150_000,
    "part": 200_000,
    "partsupp": 800_000,
    "orders": 1_500_000,
    "lineitem": 6_001_215,
}


def verify(binary, database, timeout=120, execute=None):
    if (type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0):
        raise ValueError("timeout must be finite and positive")
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    checked = []
    for table, expected in SF1_ROWS.items():
        raw = execute(f"SELECT count(*) FROM {table}")
        if not isinstance(raw, bytes):
            raise ValueError(f"{table}: count result must be bytes")
        value = raw.decode("utf-8").strip()
        if re.fullmatch(r"(0|[1-9][0-9]*)", value) is None:
            raise ValueError(f"{table}: invalid count result {value!r}")
        actual = int(value)
        if actual != expected:
            raise ValueError(f"{table}: SF1 expected {expected} rows, found {actual}")
        checked.append({"table": table, "rows": actual})
    return {"scale_factor": 1, "tables_checked": len(checked),
            "cardinality_verified": True, "tables": checked,
            "logical_data_verified": False, "official_tpch_score": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = verify(args.binary, args.database, args.timeout)
    except (OSError, UnicodeError, RuntimeError, ValueError) as error:
        parser.exit(1, f"TPC-H SF1 row audit failed: {error}\n")
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

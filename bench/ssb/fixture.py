#!/usr/bin/env python3
"""Generate a bounded SSB-inspired two-table star-schema fixture.

The output is synthetic. Full SSB uses five tables and more complex SQL.
"""
import argparse
import random
import sqlite3
from contextlib import closing
from pathlib import Path

MAX_FACTS = 1_000_000
DATE_KEYS = tuple((year * 10000 + month * 100 + 1, year, month)
                  for year in range(1993, 1998) for month in range(1, 7))
TABLES = {
    "date_dim": (
        ("d_datekey", "UInt64"),
        ("d_year", "UInt64"),
        ("d_monthnuminyear", "UInt64"),
    ),
    "lineorder": (
        ("lo_orderkey", "UInt64"),
        ("lo_linenumber", "UInt64"),
        ("lo_orderdate", "UInt64"),
        ("lo_revenue", "UInt64"),
        ("lo_quantity", "UInt64"),
        ("lo_discount", "UInt64"),
    ),
}


def write_table(root, name, records):
    names = [column for column, _ in TABLES[name]]
    path = root / (name + ".tsv")
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write("\t".join(names) + "\n")
        for record in records:
            handle.write("\t".join(map(str, record)) + "\n")
    return path


def fact_records(rows, seed):
    """Replay deterministic fact rows from bounded generator memory."""
    rng = random.Random(seed)
    for index in range(rows):
        yield (
            index + 1, 1, rng.choice(DATE_KEYS)[0],
            rng.randint(100, 1000000), rng.randint(1, 50), rng.randint(0, 10)
        )


def generate(root, rows=2000, seed=42):
    if not 1 <= rows <= MAX_FACTS:
        raise ValueError(f"lineorder rows must be 1..{MAX_FACTS}")
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    write_table(root, "date_dim", DATE_KEYS)
    write_table(root, "lineorder", fact_records(rows, seed))
    with closing(sqlite3.connect(root / "reference.sqlite")) as connection, connection:
        for table, records in (
            ("date_dim", DATE_KEYS), ("lineorder", fact_records(rows, seed))
        ):
            connection.execute("DROP TABLE IF EXISTS " + table)
            connection.execute("CREATE TABLE " + table + " (" +
                               ", ".join(name + " INTEGER" for name, _ in TABLES[table]) + ")")
            connection.executemany("INSERT INTO " + table + " VALUES (" +
                                   ",".join("?" for _ in TABLES[table]) + ")", records)
            create = "CREATE TABLE " + table + " (" + ", ".join(
                name + " " + kind for name, kind in TABLES[table]
            ) + ");\n"
            (root / (table + ".sql")).write_text(create, encoding="utf-8")
    return {"rows": rows, "dimension_rows": len(DATE_KEYS),
            "seed": seed, "official_ssb": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--rows", type=int, default=2000)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args(argv)
    try:
        result = generate(args.output_dir, args.rows, args.seed)
    except (OSError, ValueError, sqlite3.Error) as error:
        parser.exit(1, f"SSB fixture failed: {error}\n")
    print(f"generated {result['rows']} fact rows and {result['dimension_rows']} date rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

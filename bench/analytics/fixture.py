#!/usr/bin/env python3
"""Deterministic ClickBench-shaped hits fixture for a supported query subset.

This is synthetic data with seven columns, NOT the official hits dataset.
"""
import argparse
import random
import sqlite3
from contextlib import closing
from datetime import date, timedelta
from pathlib import Path

FIELDS = (
    ("AdvEngineID", "UInt64", "INTEGER"),
    ("ResolutionWidth", "UInt64", "INTEGER"),
    ("UserID", "UInt64", "INTEGER"),
    ("SearchPhrase", "String", "TEXT"),
    ("EventDate", "Date", "TEXT"),
    ("RegionID", "UInt64", "INTEGER"),
    ("MobilePhoneModel", "String", "TEXT"),
)


def records(count, seed):
    rng = random.Random(seed)
    start = date(2013, 7, 1)
    for index in range(count):
        engine = rng.randrange(0, 5)
        yield (
            engine, rng.choice((320, 768, 1024, 1440, 1920)),
            rng.randrange(1, max(2, count // 3)), f"term-{rng.randrange(0, 97)}",
            (start + timedelta(days=rng.randrange(31))).isoformat(),
            rng.randrange(0, 10), f"model-{rng.randrange(0, 8)}",
        )


def insert_batch(connection, batch):
    connection.executemany(
        "INSERT INTO hits VALUES (" + ",".join("?" for _ in FIELDS) + ")",
        batch,
    )


def generate(directory, rows=10000, seed=42):
    if type(rows) is not int or not 1 <= rows <= 1000000:
        raise ValueError("rows must be an integer in 1..1000000")
    if type(seed) is not int or not 0 <= seed <= (1 << 64) - 1:
        raise ValueError("seed must be an unsigned 64-bit integer")
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    names = [name for name, _, _ in FIELDS]
    tsv = directory / "hits.tsv"
    db = directory / "hits.sqlite"
    with closing(sqlite3.connect(db)) as connection, connection, tsv.open("w", encoding="utf-8", newline="\n") as stream:
        connection.execute("DROP TABLE IF EXISTS hits")
        connection.execute("CREATE TABLE hits (" +
                           ", ".join(f"{name} {kind}" for name, _, kind in FIELDS) + ")")
        stream.write("\t".join(names) + "\n")
        batch = []
        tsv_batch = []
        for record in records(rows, seed):
            tsv_batch.append("\t".join(map(str, record)) + "\n")
            batch.append(record)
            if len(batch) == 512:
                stream.write("".join(tsv_batch))
                tsv_batch.clear()
                insert_batch(connection, batch)
                batch.clear()
        if batch:
            stream.write("".join(tsv_batch))
            insert_batch(connection, batch)
    (directory / "create.sql").write_text(
        "CREATE TABLE hits (" + ", ".join(f"{name} {kind}" for name, kind, _ in FIELDS) + ");\n",
        encoding="utf-8",
    )
    escaped = tsv.resolve().as_posix().replace("'", "''")
    (directory / "copy.sql").write_text(
        f"COPY hits FROM '{escaped}' WITH (FORMAT TSV, HEADER TRUE);\n", encoding="utf-8"
    )
    return {"rows": rows, "seed": seed, "official_clickbench": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--rows", type=int, default=10000)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args(argv)
    try:
        result = generate(args.output_dir, args.rows, args.seed)
    except (ValueError, OSError) as error:
        parser.exit(1, f"fixture generation failed: {error}\n")
    print(f"generated {result['rows']} synthetic hits rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

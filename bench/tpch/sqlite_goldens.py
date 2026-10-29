#!/usr/bin/env python3
"""Generate reference TSV for the subset of TPC-H SQL that SQLite supports."""
import argparse
import json
import sqlite3
from contextlib import closing
from pathlib import Path

from run import load_queries, select_queries


def tsv_cell(value, null_token="NULL"):
    if value is None:
        return null_token
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        return format(value, ".17g")
    if not isinstance(value, str):
        raise ValueError(f"unsupported SQLite result type: {type(value).__name__}")
    return (value.replace("\\", "\\\\").replace("\t", "\\t")
            .replace("\n", "\\n").replace("\r", "\\r"))


def write_one(connection, sql, destination, max_rows=100_000, null_token="NULL",
              max_output_bytes=16 * 1024 * 1024):
    if type(max_rows) is not int or max_rows < 1:
        raise ValueError("max_rows must be positive")
    if type(max_output_bytes) is not int or max_output_bytes < 1:
        raise ValueError("max_output_bytes must be positive")
    destination = Path(destination)
    temporary = destination.with_name(destination.name + ".tmp")
    count = 0
    written = 0
    try:
        cursor = connection.execute(sql)
        with temporary.open("w", encoding="utf-8", newline="\n") as output:
            for row in cursor:
                if count >= max_rows:
                    raise ValueError(f"reference exceeds {max_rows} output rows")
                rendered = "\t".join(tsv_cell(value, null_token) for value in row) + "\n"
                written += len(rendered.encode("utf-8"))
                if written > max_output_bytes:
                    raise ValueError("reference exceeds output byte limit")
                output.write(rendered)
                count += 1
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
    return count


def generate(database, queries_dir, output_dir, only=None,
             max_rows=100_000, overwrite=False, null_token="NULL"):
    database = Path(database)
    if not database.is_file():
        raise ValueError("SQLite reference database does not exist")
    if null_token not in ("NULL", r"\N"):
        raise ValueError("null token must be NULL or \\N")
    queries = select_queries(load_queries(Path(queries_dir)), only)
    output_dir = Path(output_dir)
    destinations = [output_dir / f"q{number:02d}.tsv" for number, _ in queries]
    if not overwrite and any(path.exists() for path in destinations):
        raise ValueError("refusing to overwrite existing golden output")
    output_dir.mkdir(parents=True, exist_ok=True)
    summary = []
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)) as connection:
        connection.execute("PRAGMA query_only = ON")
        for (number, sql), destination in zip(queries, destinations):
            try:
                count = write_one(connection, sql, destination, max_rows, null_token)
            except (sqlite3.DatabaseError, ValueError) as error:
                raise ValueError(f"q{number:02d}: {error}") from error
            summary.append({"query": number, "rows": count, "path": str(destination)})
    return {"engine": "sqlite", "official_tpch_score": False,
            "reference_format": "tsv-no-header", "results": summary}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--queries-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--only")
    parser.add_argument("--max-rows", type=int, default=100_000)
    parser.add_argument("--overwrite", action="store_true")
    parser.add_argument("--null-token", choices=("NULL", r"\N"), default="NULL")
    args = parser.parse_args(argv)
    try:
        report = generate(args.database, args.queries_dir, args.output_dir, args.only,
                          args.max_rows, args.overwrite, args.null_token)
    except (OSError, ValueError) as error:
        parser.exit(1, f"SQLite reference failed: {error}\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

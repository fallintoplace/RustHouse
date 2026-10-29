"""Export a known contiguous UInt64 ID interval without SQL OFFSET.

Usage: python -m bench.scale.keyset_export --binary target/release/rusthouse
       --database ./db --table events --rows 1000000000 --output /data/ids.tsv
This verifies sequential IDs, not arbitrary sort orders or arbitrary primary keys.
"""
import argparse
import hashlib
import json
import math
import os
import re
import subprocess
import time
from pathlib import Path


def identifier(value):
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", value):
        raise ValueError("table and column names must be simple SQL identifiers")
    return value


def page_sql(table, column, first, count):
    identifier(table)
    identifier(column)
    if (type(first) is not int or type(count) is not int
            or first < 0 or count < 1 or first + count > 2**64):
        raise ValueError("invalid UInt64 page")
    high = first + count
    upper = f"{column} < {high}" if high < 2**64 else f"{column} <= {2**64 - 1}"
    return (f"SELECT {column} FROM {table} WHERE {column} >= {first} "
            f"AND {upper} ORDER BY {column} ASC LIMIT {count + 1}")


def query(binary, database, statement, timeout):
    result = subprocess.run(
        [str(binary), "sql", statement, "--database", str(database),
         "--format", "tsv-no-header"],
        capture_output=True, check=False, timeout=timeout,
    )
    if result.returncode:
        raise RuntimeError(result.stderr.decode("utf-8", "replace")[-500:])
    return result.stdout


def export(binary, database, table, output, start, rows, page_rows=8192, timeout=300,
           column="id", execute=None, max_total_seconds=None):
    if type(start) is not int or type(rows) is not int or start < 0 or rows < 1:
        raise ValueError("start and rows must be positive UInt64 bounds")
    if (start + rows > 2**64 or type(page_rows) is not int
            or not 1 <= page_rows <= 65535):
        raise ValueError("range overflow or invalid page size (1..65535)")
    if type(timeout) not in (float, int) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    if (max_total_seconds is not None and
            (type(max_total_seconds) not in (int, float)
             or not math.isfinite(max_total_seconds) or max_total_seconds <= 0)):
        raise ValueError("max_total_seconds must be finite and positive")
    output = Path(output)
    if output.exists():
        raise ValueError("refusing to overwrite an existing export")
    execute = execute or (lambda statement: query(binary, database, statement, timeout))
    temporary = output.with_name("." + output.name + f".{os.getpid()}.pending")
    digest = hashlib.sha256()
    pages = 0
    stop = start + rows
    started = time.monotonic()
    try:
        with temporary.open("xb") as sink:
            for first in range(start, stop, page_rows):
                if (max_total_seconds is not None
                        and time.monotonic() - started > max_total_seconds):
                    raise TimeoutError("total export deadline exceeded")
                count = min(page_rows, stop - first)
                data = execute(page_sql(table, column, first, count))
                # Compare the complete expected page in one operation.
                # Avoid one Python bytes allocation and one splitline per ID.
                expected = "".join(f"{number}\n" for number in
                                   range(first, first + count)).encode("ascii")
                if data != expected:
                    raise ValueError(f"missing, duplicated or unordered ID near {first}")
                sink.write(data)
                digest.update(data)
                pages += 1
            sink.flush()
            os.fsync(sink.fileno())
        if output.exists():
            raise ValueError("refusing to overwrite an existing export")
        os.link(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)
    return {"rows": rows, "pages": pages, "start": start,
            "sha256": digest.hexdigest(), "output": str(output)}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", required=True, type=Path)
    p.add_argument("--database", required=True, type=Path)
    p.add_argument("--table", required=True)
    p.add_argument("--column", default="id")
    p.add_argument("--start", type=int, default=0)
    p.add_argument("--rows", required=True, type=int)
    p.add_argument("--page-rows", type=int, default=8192)
    p.add_argument("--output", required=True, type=Path)
    p.add_argument("--timeout", type=float, default=300)
    p.add_argument("--max-total-seconds", type=float)
    a = p.parse_args(argv)
    if not math.isfinite(a.timeout) or a.timeout <= 0:
        p.error("--timeout must be finite and positive")
    try:
        report = export(a.binary, a.database, a.table, a.output, a.start,
                        a.rows, a.page_rows, a.timeout, a.column,
                        max_total_seconds=a.max_total_seconds)
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        p.exit(1, f"keyset export failed: {error}\n")
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

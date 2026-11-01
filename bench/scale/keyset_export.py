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
import tempfile
import time
from pathlib import Path


def identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]{0,127}", value):
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
    # The CLI may be noisy. Bound memory for both successful output and errors.
    with tempfile.TemporaryFile(mode="w+b") as output, tempfile.TemporaryFile(mode="w+b") as errors:
        extra = {}
        if os.name == "posix":
            import resource

            def cap_output_file():
                _, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
                limit = (2 * 1024 * 1024 if hard == resource.RLIM_INFINITY
                         else min(2 * 1024 * 1024, hard))
                resource.setrlimit(resource.RLIMIT_FSIZE, (limit, hard))

            extra["preexec_fn"] = cap_output_file
        result = subprocess.run(
            [str(binary), "sql", statement, "--database", str(database),
             "--format", "tsv-no-header"],
            stdout=output, stderr=errors, check=False, timeout=timeout, **extra,
        )
        if result.returncode:
            output.seek(0, os.SEEK_END)
            if output.tell() >= 2 * 1024 * 1024:
                raise ValueError("keyset SQL page output exceeds 2 MiB")
            errors.seek(0, os.SEEK_END)
            errors.seek(max(0, errors.tell() - 500))
            raise RuntimeError(errors.read(500).decode("utf-8", "replace"))
        output.seek(0)
        payload = output.read(2 * 1024 * 1024 + 1)
        if len(payload) > 2 * 1024 * 1024:
            raise ValueError("keyset SQL page output exceeds 2 MiB")
        return payload

def export(binary, database, table, output, start, rows, page_rows=8192, timeout=300,
           column="id", execute=None, max_total_seconds=None,
           expected_sha256=None):
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
    if expected_sha256 is not None and (
            not isinstance(expected_sha256, str)
            or not re.fullmatch(r"[0-9a-f]{64}", expected_sha256)):
        raise ValueError("expected_sha256 must be lowercase SHA256")
    identifier(table)
    identifier(column)
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise ValueError("refusing to overwrite an existing export")
    use_default_query = execute is None
    temporary = None
    digest = hashlib.sha256()
    pages = 0
    started = time.monotonic()
    try:
        with tempfile.NamedTemporaryFile(
            mode="w+b", dir=output.parent, prefix=f".{output.name}.",
            suffix=".pending", delete=False,
        ) as sink:
            temporary = Path(sink.name)
            for first in range(start, start + rows, page_rows):
                remaining = None
                if max_total_seconds is not None:
                    remaining = max_total_seconds - (time.monotonic() - started)
                    if remaining <= 0:
                        raise TimeoutError("total export deadline exceeded")
                count = min(page_rows, start + rows - first)
                statement = page_sql(table, column, first, count)
                if use_default_query:
                    page_timeout = min(timeout, remaining) if remaining is not None else timeout
                    data = query(binary, database, statement, page_timeout)
                else:
                    data = execute(statement)
                if (max_total_seconds is not None
                        and time.monotonic() - started > max_total_seconds):
                    raise TimeoutError("total export deadline exceeded")
                # Compare the complete expected page in one operation.
                # Avoid one Python bytes allocation and one splitline per ID.
                # Convert whole integer ranges with map(str) rather than
                # dispatching an f-string for every row in billion-ID exports.
                expected = ("\n".join(map(str, range(first, first + count))) + "\n").encode("ascii")
                if data != expected:
                    raise ValueError(f"missing, duplicated or unordered ID near {first}")
                sink.write(data)
                digest.update(data)
                pages += 1
            sink.flush()
            os.fsync(sink.fileno())
        if expected_sha256 is not None and digest.hexdigest() != expected_sha256:
            raise ValueError("keyset export SHA256 differs from expected_sha256")
        if output.exists() or output.is_symlink():
            raise ValueError("refusing to overwrite an existing export")
        os.link(temporary, output)
        if os.name == "posix":
            descriptor = os.open(output.parent, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    return {"rows": rows, "pages": pages, "start": start,
            "sha256": digest.hexdigest(), "verified_sha256": expected_sha256 is not None,
            "output": str(output)}


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
    p.add_argument("--expected-sha256")
    a = p.parse_args(argv)
    if not math.isfinite(a.timeout) or a.timeout <= 0:
        p.error("--timeout must be finite and positive")
    try:
        report = export(a.binary, a.database, a.table, a.output, a.start,
                        a.rows, a.page_rows, a.timeout, a.column,
                        max_total_seconds=a.max_total_seconds,
                        expected_sha256=a.expected_sha256)
    except (OSError, RuntimeError, ValueError, TimeoutError, subprocess.TimeoutExpired) as error:
        p.exit(1, f"keyset export failed: {error}\n")
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

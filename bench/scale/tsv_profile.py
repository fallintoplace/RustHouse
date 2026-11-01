"""Stream-validate TSV record structure, counts and SHA256 without loading files."""
import argparse
import hashlib
import json
import os
import re
import stat

from bench.common import file_change_time_ns
from bench.scale.tsv_shards import validate_copy_tsv_record
from pathlib import Path

MAX_RECORD_BYTES = 8 * 1024 * 1024


def profile(path, *, columns=1, header=False, min_rows=1,
            verify_sorted_uint64=False, expected_sha256=None,
            verify_copy_escapes=False):
    if type(columns) is not int or not 1 <= columns <= 1024:
        raise ValueError("columns must be in 1..1024")
    if type(min_rows) is not int or min_rows < 1:
        raise ValueError("min_rows must be positive")
    if type(header) is not bool:
        raise ValueError("header must be a boolean")
    if type(verify_sorted_uint64) is not bool or (verify_sorted_uint64 and columns != 1):
        raise ValueError("sorted UInt64 verification requires one column")
    if type(verify_copy_escapes) is not bool or (verify_copy_escapes and columns > 128):
        raise ValueError("verify_copy_escapes requires at most 128 columns and a boolean flag")
    if expected_sha256 is not None and (
            not isinstance(expected_sha256, str)
            or not re.fullmatch(r"[0-9a-f]{64}", expected_sha256)):
        raise ValueError("expected_sha256 must be a lowercase SHA256 hash")
    previous = None
    digest = hashlib.sha256()
    records = 0
    bytes_read = 0
    source = Path(path)
    before = source.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("expected a regular, non-symlinked TSV file")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(source, flags)
    with os.fdopen(fd, "rb") as stream:
        opened = os.fstat(stream.fileno())
        if (not stat.S_ISREG(opened.st_mode)
                or (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino)):
            raise ValueError("TSV file changed before opening")
        while True:
            raw = stream.readline(MAX_RECORD_BYTES + 1)
            if not raw:
                break
            if len(raw) > MAX_RECORD_BYTES or not raw.endswith(b"\n"):
                raise ValueError(f"record {records + 1}: oversized or truncated")
            row = raw.removesuffix(b"\n").removesuffix(b"\r")
            if not row or row.count(b"\t") != columns - 1:
                raise ValueError(f"record {records + 1}: invalid column count or empty row")
            if b"\x00" in row:
                raise ValueError(f"record {records + 1}: raw NUL byte")
            if verify_copy_escapes and not (header and records == 0):
                # COPY validates escaped values as UTF-8, not only raw TSV.
                # Reuse the shard splitter's exact escape grammar.
                validate_copy_tsv_record(raw)
            # Sorted numeric records are validated as ASCII bytes below.
            # Skip a redundant Unicode allocation per row in large fixtures.
            if not (verify_sorted_uint64 and not (header and records == 0)):
                row.decode("utf-8", errors="strict")
            if verify_sorted_uint64 and not (header and records == 0):
                if (not row.isascii() or not row.isdigit() or len(row) > 20
                        or (len(row) > 1 and row.startswith(b"0"))):
                    raise ValueError(f"record {records + 1}: invalid UInt64 key")
                value = int(row)
                if value > (1 << 64) - 1:
                    raise ValueError(f"record {records + 1}: UInt64 overflow")
                if previous is not None and value <= previous:
                    raise ValueError(f"record {records + 1}: keys are not strictly increasing")
                previous = value
            digest.update(raw)
            bytes_read += len(raw)
            records += 1
        finished = os.fstat(stream.fileno())
        if (opened.st_size, opened.st_mtime_ns, file_change_time_ns(opened)) != (finished.st_size, finished.st_mtime_ns, file_change_time_ns(finished)):
            raise ValueError("TSV file changed while profiling")
    after = source.lstat()
    if ((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, file_change_time_ns(before))
            != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, file_change_time_ns(after))):
        raise ValueError("TSV file changed while profiling")
    rows = records - int(header)
    if rows < min_rows:
        raise ValueError(f"input has {rows} data rows; require {min_rows}")
    actual_sha256 = digest.hexdigest()
    if expected_sha256 is not None and actual_sha256 != expected_sha256:
        raise ValueError("TSV fingerprint differs from expected_sha256")
    result = {"format": "tsv", "rows": rows, "header": header, "columns": columns,
              "bytes": bytes_read, "sha256": actual_sha256}
    if expected_sha256 is not None:
        result["sha256_verified"] = True
    if verify_copy_escapes:
        result["copy_escapes_verified"] = True
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--columns", type=int, default=1)
    parser.add_argument("--header", action="store_true")
    parser.add_argument("--min-rows", type=int, default=1)
    parser.add_argument("--verify-sorted-uint64", action="store_true")
    parser.add_argument("--verify-copy-escapes", action="store_true",
                        help="Also validate decoded COPY TSV escapes and UTF-8")
    parser.add_argument("--expected-sha256")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = profile(args.input, columns=args.columns, header=args.header,
                         min_rows=args.min_rows,
                         verify_sorted_uint64=args.verify_sorted_uint64,
                         expected_sha256=args.expected_sha256,
                         verify_copy_escapes=args.verify_copy_escapes)
        content = json.dumps(result, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(content, encoding="utf-8")
    except (OSError, ValueError, UnicodeError) as error:
        parser.exit(1, f"TSV profile failed: {error}\n")
    print(content, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

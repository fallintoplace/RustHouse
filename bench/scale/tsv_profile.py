"""Stream-validate TSV record structure, counts and SHA256 without loading files."""
import argparse
import hashlib
import json
from pathlib import Path

MAX_RECORD_BYTES = 8 * 1024 * 1024


def profile(path, *, columns=1, header=False, min_rows=1,
            verify_sorted_uint64=False):
    if type(columns) is not int or not 1 <= columns <= 1024:
        raise ValueError("columns must be in 1..1024")
    if type(min_rows) is not int or min_rows < 1:
        raise ValueError("min_rows must be positive")
    if type(header) is not bool:
        raise ValueError("header must be a boolean")
    if type(verify_sorted_uint64) is not bool or (verify_sorted_uint64 and columns != 1):
        raise ValueError("sorted UInt64 verification requires one column")
    previous = None
    digest = hashlib.sha256()
    records = 0
    bytes_read = 0
    source = Path(path)
    if not source.is_file() or source.is_symlink():
        raise ValueError("expected a regular, non-symlinked TSV file")
    with source.open("rb") as stream:
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
            row.decode("utf-8", errors="strict")
            if verify_sorted_uint64 and not (header and records == 0):
                if not row.isascii() or not row.isdigit() or len(row) > 20:
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
    rows = records - int(header)
    if rows < min_rows:
        raise ValueError(f"input has {rows} data rows; require {min_rows}")
    return {"format": "tsv", "rows": rows, "header": header, "columns": columns,
            "bytes": bytes_read, "sha256": digest.hexdigest()}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--columns", type=int, default=1)
    parser.add_argument("--header", action="store_true")
    parser.add_argument("--min-rows", type=int, default=1)
    parser.add_argument("--verify-sorted-uint64", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = profile(args.input, columns=args.columns, header=args.header,
                         min_rows=args.min_rows,
                         verify_sorted_uint64=args.verify_sorted_uint64)
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

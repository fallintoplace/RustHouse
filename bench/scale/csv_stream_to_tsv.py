"""Stream large CSV inputs into RustHouse COPY-compatible TSV.

Usage: python -m bench.scale.csv_stream_to_tsv input.csv output.tsv --header
The default maps empty CSV fields to empty strings; --empty-as-null maps them
to TSV NULLs. CSV quote provenance for empty fields is not retained.
"""
import argparse
import csv
import os
import secrets
import stat
from pathlib import Path


ESCAPE_TRANSLATION = str.maketrans({
    "\\": r"\\", "\t": r"\t", "\n": r"\n", "\r": r"\r",
    "\x00": r"\0", "\b": r"\b", "\f": r"\f", "\v": r"\v",
})


def escape_field(value, empty_as_null=False):
    if value == "" and empty_as_null:
        return r"\N"
    # Translate in CPython instead of rebuilding a map and iterating every
    # character in Python for each field of huge COPY inputs.
    return value.translate(ESCAPE_TRANSLATION)


def utf8_length(value):
    """Exact encoded byte length without allocating for ASCII fields."""
    return len(value) if value.isascii() else len(value.encode("utf-8"))


def convert(source, destination, *, header=False, empty_as_null=False,
            max_field_bytes=8 * 1024 * 1024, skip_header=False,
            expected_columns=None):
    source = Path(source)
    before = source.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("CSV source must be a regular non-symlinked file")
    if Path(source).resolve() == Path(destination).resolve():
        raise ValueError("input and output must be different files")
    if skip_header and not header:
        raise ValueError("skip_header requires header=True")
    # COPY TSV rows cannot exceed 8 MiB. Larger CSV field limits cannot
    # produce a valid row and can overflow csv.field_size_limit's C integer.
    if type(max_field_bytes) is not int or not 1 <= max_field_bytes <= 8 * 1024 * 1024:
        raise ValueError("max_field_bytes must be a positive integer in 1..8388608")
    if expected_columns is not None and (
            type(expected_columns) is not int or not 1 <= expected_columns <= 128):
        raise ValueError("expected_columns must be in 1..128")
    old_limit = csv.field_size_limit()
    csv.field_size_limit(max_field_bytes)
    count = 0
    width = None
    destination = Path(destination)
    pending = destination.with_name(
        "." + destination.name + f".{os.getpid()}.{secrets.token_hex(8)}.pending")
    pending_created = False
    try:
        if destination.exists():
            raise ValueError("refusing to overwrite an existing output")
        # The preflight can race with an input replacement. Open by descriptor
        # without following a final symlink, then compare the opened inode.
        flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(source, flags)
        with os.fdopen(descriptor, "r", encoding="utf-8-sig", newline="") as input_file:
            opened = os.fstat(input_file.fileno())
            if ((opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino)
                    or not stat.S_ISREG(opened.st_mode)):
                raise ValueError("CSV source changed while opening")
            with pending.open("x", encoding="utf-8", newline="") as output_file:
                pending_created = True
                reader = csv.reader(input_file, strict=True)
                for fields in reader:
                    if width is None:
                        width = len(fields)
                        if width < 1 or width > 128:
                            raise ValueError("CSV must contain 1..128 columns")
                        if expected_columns is not None and width != expected_columns:
                            raise ValueError("CSV columns differ from expected_columns")
                    elif len(fields) != width:
                        raise ValueError(f"CSV column count mismatch at record {count + 1}")
                    if any(utf8_length(value) > max_field_bytes for value in fields):
                        raise ValueError("CSV record contains an oversized UTF-8 field")
                    if skip_header and header and count == 0:
                        count += 1
                        continue
                    rendered = "\t".join(escape_field(value, empty_as_null)
                                         for value in fields) + "\n"
                    if utf8_length(rendered) > 8 * 1024 * 1024:
                        raise ValueError("COPY TSV record exceeds 8 MiB")
                    output_file.write(rendered)
                    count += 1
                output_file.flush()
                os.fsync(output_file.fileno())
            finished = os.fstat(input_file.fileno())
            after = source.lstat()
            identity = lambda metadata: (
                metadata.st_dev, metadata.st_ino, metadata.st_size,
                metadata.st_mtime_ns, metadata.st_ctime_ns,
            )
            if identity(before) != identity(opened) or (
                identity(opened) != identity(finished)
                or identity(finished) != identity(after)
            ):
                raise ValueError("CSV source changed during conversion")
        if destination.exists():
            raise ValueError("refusing to overwrite an existing output")
        os.link(pending, destination)
        if os.name == "posix":
            # Publishing a hard link needs a parent-directory fsync to survive
            # a crash, even when the TSV payload has already been fsynced.
            descriptor = os.open(destination.parent, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
    finally:
        if pending_created:
            pending.unlink(missing_ok=True)
        csv.field_size_limit(old_limit)
    return {"records": count, "data_rows": count - int(header and count > 0),
            "columns": width or 0, "header": header,
            "empty_as_null": empty_as_null}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--header", action="store_true")
    parser.add_argument("--skip-header", action="store_true", help="omit CSV header from COPY TSV")
    parser.add_argument("--empty-as-null", action="store_true")
    parser.add_argument("--expected-columns", type=int)
    args = parser.parse_args(argv)
    try:
        report = convert(args.source, args.destination, header=args.header,
                         empty_as_null=args.empty_as_null, skip_header=args.skip_header,
                         expected_columns=args.expected_columns)
    except (OSError, csv.Error, UnicodeError, ValueError) as error:
        parser.exit(1, f"CSV conversion failed: {error}\n")
    print(f"{report['data_rows']} data rows written to {args.destination}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

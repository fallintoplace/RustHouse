"""Stream large CSV inputs into RustHouse COPY-compatible TSV.

Usage: python -m bench.scale.csv_stream_to_tsv input.csv output.tsv --header
The default maps empty CSV fields to empty strings; --empty-as-null maps them
to TSV NULLs. CSV quote provenance for empty fields is not retained.
"""
import argparse
import csv
import os
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


def convert(source, destination, *, header=False, empty_as_null=False,
            max_field_bytes=8 * 1024 * 1024):
    if Path(source).resolve() == Path(destination).resolve():
        raise ValueError("input and output must be different files")
    if max_field_bytes < 1:
        raise ValueError("max_field_bytes must be positive")
    old_limit = csv.field_size_limit()
    csv.field_size_limit(max_field_bytes)
    count = 0
    width = None
    destination = Path(destination)
    pending = destination.with_name("." + destination.name + f".{os.getpid()}.pending")
    try:
        if destination.exists():
            raise ValueError("refusing to overwrite an existing output")
        with open(source, "r", encoding="utf-8-sig", newline="") as input_file:
            with pending.open("x", encoding="utf-8", newline="") as output_file:
                reader = csv.reader(input_file, strict=True)
                for fields in reader:
                    if width is None:
                        width = len(fields)
                        if width < 1 or width > 128:
                            raise ValueError("CSV must contain 1..128 columns")
                    elif len(fields) != width:
                        raise ValueError(f"CSV column count mismatch at record {count + 1}")
                    if any(len(value.encode("utf-8")) > max_field_bytes for value in fields):
                        raise ValueError("CSV record contains an oversized UTF-8 field")
                    rendered = "\t".join(escape_field(value, empty_as_null)
                                         for value in fields) + "\n"
                    if len(rendered.encode("utf-8")) > 8 * 1024 * 1024:
                        raise ValueError("COPY TSV record exceeds 8 MiB")
                    output_file.write(rendered)
                    count += 1
                output_file.flush()
                os.fsync(output_file.fileno())
        if destination.exists():
            raise ValueError("refusing to overwrite an existing output")
        os.link(pending, destination)
    finally:
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
    parser.add_argument("--empty-as-null", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = convert(args.source, args.destination, header=args.header,
                         empty_as_null=args.empty_as_null)
    except (OSError, csv.Error, UnicodeError, ValueError) as error:
        parser.exit(1, f"CSV conversion failed: {error}\n")
    print(f"{report['data_rows']} data rows written to {args.destination}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

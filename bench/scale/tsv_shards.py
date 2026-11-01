"""Split a headerless TSV file into bounded, hash-verified chunks.

Does not parse SQL or modify a database. Output directory is atomically published.
"""
import argparse
import hashlib
import json
import os
import shutil
import stat
import tempfile
from pathlib import Path

from bench.common import file_change_time_ns

MAX_RECORD_BYTES = 8 * 1024 * 1024
MAX_SHARDS = 16_384

# Keep shard preflight compatible with the bounded RustHouse COPY TSV reader.
_SIMPLE_ESCAPES = {ord("\\"): ord("\\"), ord("t"): 9, ord("n"): 10,
                   ord("r"): 13, ord("0"): 0, ord("b"): 8,
                   ord("f"): 12, ord("v"): 11}


def validate_copy_tsv_record(record):
    """Reject records COPY would fail to decode before publishing any shards."""
    line = record[:-1].removesuffix(b"\r")
    if b"\r" in line or b"\x00" in line:
        raise ValueError("COPY TSV record contains a raw control byte")
    if line.count(b"\t") >= 128:
        raise ValueError("COPY TSV record exceeds 128 columns")
    # COPY requires UTF-8 even before decoding TSV escape sequences.
    try:
        line.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ValueError("COPY TSV record is not UTF-8") from error
    if b"\\" not in line:
        return
    for field in line.split(b"\t"):
        if field == b"\\N" or b"\\" not in field:
            continue
        decoded = bytearray()
        cursor = 0
        while cursor < len(field):
            if field[cursor] != 92:
                end = field.find(b"\\", cursor)
                if end < 0:
                    end = len(field)
                decoded.extend(field[cursor:end])
                cursor = end
                continue
            cursor += 1
            if cursor >= len(field):
                raise ValueError("incomplete COPY TSV escape")
            escaped = field[cursor]
            if escaped == ord("x"):
                if cursor + 2 >= len(field):
                    raise ValueError("incomplete COPY TSV hex escape")
                pair = field[cursor + 1:cursor + 3]
                if any(digit not in b"0123456789abcdefABCDEF" for digit in pair):
                    raise ValueError("invalid COPY TSV hex escape")
                decoded.append(int(pair, 16))
                cursor += 3
            else:
                value = _SIMPLE_ESCAPES.get(escaped)
                if value is None:
                    raise ValueError("invalid COPY TSV escape")
                decoded.append(value)
                cursor += 1
        try:
            decoded.decode("utf-8")
        except UnicodeDecodeError as error:
            raise ValueError("COPY TSV escape decodes to invalid UTF-8") from error


def split_tsv(source, output_dir, *, rows_per_chunk=65536,
              max_bytes_per_chunk=None, max_total_rows=None):
    source, output_dir = Path(source), Path(output_dir)
    if type(rows_per_chunk) is not int or not 1 <= rows_per_chunk <= 1_000_000:
        raise ValueError("rows_per_chunk must be in 1..1000000")
    if max_bytes_per_chunk is not None and (
            type(max_bytes_per_chunk) is not int or max_bytes_per_chunk < 1):
        raise ValueError("max_bytes_per_chunk must be a positive integer")
    if max_total_rows is not None and (
            type(max_total_rows) is not int or max_total_rows < 1):
        raise ValueError("max_total_rows must be a positive integer")
    source_before = source.lstat()
    if not stat.S_ISREG(source_before.st_mode):
        raise ValueError("input must be a regular, non-symlinked TSV file")
    if output_dir.exists() or output_dir.is_symlink():
        raise FileExistsError(f"output directory exists: {output_dir}")
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=".rusthouse-shards-", dir=output_dir.parent))
    shards = []
    total_rows = 0
    current = None
    pending = bytearray()

    def flush_pending():
        if pending:
            current["file"].write(pending)
            current["sha256"].update(pending)
            pending.clear()

    def finish_current():
        flush_pending()
        current["file"].flush()
        os.fsync(current["file"].fileno())
        current["file"].close()
        del current["file"]

    try:
        flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(source, flags)
        with os.fdopen(descriptor, "rb") as stream:
            opened = os.fstat(stream.fileno())
            if (not stat.S_ISREG(opened.st_mode)
                    or (opened.st_dev, opened.st_ino, opened.st_size,
                        opened.st_mtime_ns, file_change_time_ns(opened)) !=
                       (source_before.st_dev, source_before.st_ino,
                        source_before.st_size, source_before.st_mtime_ns,
                        file_change_time_ns(source_before))):
                raise ValueError("TSV source changed before opening")
            while True:
                record = stream.readline(MAX_RECORD_BYTES + 1)
                if not record:
                    break
                if max_total_rows is not None and total_rows >= max_total_rows:
                    raise ValueError("TSV input exceeds max_total_rows")
                if len(record) > MAX_RECORD_BYTES or not record.endswith(b"\n"):
                    raise ValueError("truncated or oversized TSV record")
                if record in (b"\n", b"\r\n"):
                    raise ValueError("empty TSV record")
                validate_copy_tsv_record(record)
                if (max_bytes_per_chunk is not None
                        and len(record) > max_bytes_per_chunk):
                    raise ValueError("TSV record exceeds max_bytes_per_chunk")
                if (current is None or current["rows"] == rows_per_chunk
                        or (max_bytes_per_chunk is not None and
                            current["bytes"] + len(record) > max_bytes_per_chunk)):
                    if current is not None:
                        finish_current()
                    if len(shards) >= MAX_SHARDS:
                        raise ValueError("shard count exceeds RustHouse's 16384-part limit")
                    filename = f"part-{len(shards):06}.tsv"
                    handle = (temporary / filename).open("wb")
                    current = {"path": filename, "rows": 0, "bytes": 0,
                               "sha256": hashlib.sha256(), "file": handle}
                    shards.append(current)
                pending.extend(record)
                if len(pending) >= 256 * 1024:
                    flush_pending()
                current["bytes"] += len(record)
                current["rows"] += 1
                total_rows += 1
            finished = os.fstat(stream.fileno())
            if ((opened.st_size, opened.st_mtime_ns, file_change_time_ns(opened))
                    != (finished.st_size, finished.st_mtime_ns, file_change_time_ns(finished))):
                raise ValueError("TSV source changed while splitting")
        source_after = source.lstat()
        if ((source_before.st_dev, source_before.st_ino,
             source_before.st_size, source_before.st_mtime_ns, file_change_time_ns(source_before))
                != (source_after.st_dev, source_after.st_ino,
                    source_after.st_size, source_after.st_mtime_ns, file_change_time_ns(source_after))):
            raise ValueError("TSV source changed while splitting")
        if current is not None:
            finish_current()
        if not shards:
            raise ValueError("input TSV has no records")
        for shard in shards:
            shard["sha256"] = shard["sha256"].hexdigest()
        manifest = {"format": "rusthouse-tsv-shards-v1", "header": False,
                    "total_rows": sum(s["rows"] for s in shards),
                    "total_bytes": sum(s["bytes"] for s in shards),
                    "shards": shards}
        with (temporary / "manifest.json").open("w", encoding="utf-8") as output:
            output.write(json.dumps(manifest, indent=2) + "\n")
            output.flush()
            os.fsync(output.fileno())
        if os.name == "posix":
            descriptor = os.open(temporary, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
        if output_dir.exists() or output_dir.is_symlink():
            raise FileExistsError(f"output directory exists: {output_dir}")
        os.replace(temporary, output_dir)
        if os.name == "posix":
            descriptor = os.open(output_dir.parent, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
        return manifest
    finally:
        if current is not None and "file" in current:
            current["file"].close()
        if temporary.exists():
            shutil.rmtree(temporary)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--rows-per-chunk", type=int, default=65536)
    parser.add_argument("--max-total-rows", type=int)
    parser.add_argument("--max-bytes-per-chunk", type=int,
                        help="Optional maximum shard file size in bytes")
    args = parser.parse_args(argv)
    try:
        report = split_tsv(args.input, args.output_dir,
                           rows_per_chunk=args.rows_per_chunk,
                           max_bytes_per_chunk=args.max_bytes_per_chunk,
                           max_total_rows=args.max_total_rows)
    except (OSError, ValueError) as error:
        parser.exit(1, f"TSV shard split failed: {error}\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

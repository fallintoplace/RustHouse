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

MAX_RECORD_BYTES = 8 * 1024 * 1024
MAX_SHARDS = 16_384


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
                        opened.st_mtime_ns, opened.st_ctime_ns) !=
                       (source_before.st_dev, source_before.st_ino,
                        source_before.st_size, source_before.st_mtime_ns,
                        source_before.st_ctime_ns)):
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
            if ((opened.st_size, opened.st_mtime_ns, opened.st_ctime_ns)
                    != (finished.st_size, finished.st_mtime_ns, finished.st_ctime_ns)):
                raise ValueError("TSV source changed while splitting")
        source_after = source.lstat()
        if ((source_before.st_dev, source_before.st_ino,
             source_before.st_size, source_before.st_mtime_ns, source_before.st_ctime_ns)
                != (source_after.st_dev, source_after.st_ino,
                    source_after.st_size, source_after.st_mtime_ns, source_after.st_ctime_ns)):
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

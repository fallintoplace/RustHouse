"""Split a headerless TSV file into bounded, hash-verified chunks.

Does not parse SQL or modify a database. Output directory is atomically published.
"""
import argparse
import hashlib
import json
import os
import shutil
import tempfile
from pathlib import Path

MAX_RECORD_BYTES = 8 * 1024 * 1024
MAX_SHARDS = 16_384


def split_tsv(source, output_dir, *, rows_per_chunk=65536,
              max_bytes_per_chunk=None):
    source, output_dir = Path(source), Path(output_dir)
    if type(rows_per_chunk) is not int or not 1 <= rows_per_chunk <= 1_000_000:
        raise ValueError("rows_per_chunk must be in 1..1000000")
    if max_bytes_per_chunk is not None and (
            type(max_bytes_per_chunk) is not int or max_bytes_per_chunk < 1):
        raise ValueError("max_bytes_per_chunk must be a positive integer")
    if output_dir.exists() or output_dir.is_symlink():
        raise FileExistsError(f"output directory exists: {output_dir}")
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=".rusthouse-shards-", dir=output_dir.parent))
    shards = []
    current = None
    pending = bytearray()

    def flush_pending():
        if pending:
            current["file"].write(pending)
            current["sha256"].update(pending)
            pending.clear()

    try:
        with source.open("rb") as stream:
            while True:
                record = stream.readline(MAX_RECORD_BYTES + 1)
                if not record:
                    break
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
                        flush_pending()
                        current["file"].close()
                        del current["file"]
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
        if current is not None:
            flush_pending()
            current["file"].close()
            del current["file"]
        if not shards:
            raise ValueError("input TSV has no records")
        for shard in shards:
            shard["sha256"] = shard["sha256"].hexdigest()
        manifest = {"format": "rusthouse-tsv-shards-v1", "header": False,
                    "total_rows": sum(s["rows"] for s in shards),
                    "total_bytes": sum(s["bytes"] for s in shards),
                    "shards": shards}
        (temporary / "manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        if output_dir.exists() or output_dir.is_symlink():
            raise FileExistsError(f"output directory exists: {output_dir}")
        os.replace(temporary, output_dir)
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
    parser.add_argument("--max-bytes-per-chunk", type=int,
                        help="Optional maximum shard file size in bytes")
    args = parser.parse_args(argv)
    try:
        report = split_tsv(args.input, args.output_dir,
                           rows_per_chunk=args.rows_per_chunk,
                           max_bytes_per_chunk=args.max_bytes_per_chunk)
    except (OSError, ValueError) as error:
        parser.exit(1, f"TSV shard split failed: {error}\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Write massive sequential UInt64 TSV fixtures without retaining rows in memory.

The output is atomically published only after the full write succeeds.
This is a data generator, not a benchmark of RustHouse query throughput.
"""
import argparse
import os
import tempfile
from pathlib import Path

MAX_UINT64 = (1 << 64) - 1


def emit_rows(stream, rows, *, start=0, step=1, batch_rows=8192, header=False):
    if type(rows) is not int or rows < 1:
        raise ValueError("rows must be positive")
    if type(start) is not int or not 0 <= start <= MAX_UINT64:
        raise ValueError("start must be UInt64")
    if type(step) is not int or step < 1:
        raise ValueError("step must be a positive integer")
    if rows - 1 > (MAX_UINT64 - start) // step:
        raise ValueError("UInt64 sequence would overflow")
    if type(batch_rows) is not int or not 1 <= batch_rows <= 1_000_000:
        raise ValueError("batch_rows must be in 1..1000000")
    if type(header) is not bool:
        raise ValueError("header must be a boolean")
    if header:
        stream.write("id\n")
    if step == 1:
        # Preserve the fast contiguous path for existing billion-row fixtures.
        stop = start + rows
        for index in range(start, stop, batch_rows):
            end = min(index + batch_rows, stop)
            stream.write("".join(f"{number}\n" for number in range(index, end)))
    else:
        for index in range(0, rows, batch_rows):
            end = min(index + batch_rows, rows)
            stream.write("".join(f"{start + number * step}\n"
                                 for number in range(index, end)))
    return rows


def generate(path, rows, *, start=0, step=1, batch_rows=8192, header=False, force=False):
    path = Path(path)
    if type(force) is not bool:
        raise ValueError("force must be a boolean")
    if (path.exists() or path.is_symlink()) and not force:
        raise FileExistsError(f"refusing to overwrite {path} without --force")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="ascii", newline="\n", dir=path.parent,
            prefix=f".{path.name}.", suffix=".tmp", delete=False,
        ) as stream:
            temporary = Path(stream.name)
            emit_rows(stream, rows, start=start, step=step, batch_rows=batch_rows, header=header)
            stream.flush()
            os.fsync(stream.fileno())
        if force:
            os.replace(temporary, path)
        else:
            os.link(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--step", type=int, default=1,
                        help="Positive UInt64 stride between consecutive IDs")
    parser.add_argument("--batch-rows", type=int, default=8192)
    parser.add_argument("--header", action="store_true")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    try:
        generate(args.output, args.rows, start=args.start, step=args.step, batch_rows=args.batch_rows,
                 header=args.header, force=args.force)
    except (OSError, ValueError) as error:
        parser.exit(1, f"TSV generation failed: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

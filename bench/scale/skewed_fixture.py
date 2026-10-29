"""Generate (id, group_id) UInt64 TSV with controlled hot-key skew.

Skew is deterministic and uses no RNG state or in-memory row map.
"""
import argparse
import os
import tempfile
from pathlib import Path


def emit(stream, rows, *, groups=1000, hot_percent=90, batch_rows=8192):
    if not 1 <= rows <= (1 << 64) - 1:
        raise ValueError("rows must be positive UInt64")
    if not 1 <= groups <= (1 << 32) or not 0 <= hot_percent <= 100:
        raise ValueError("invalid groups or hot_percent")
    if not 1 <= batch_rows <= 1_000_000:
        raise ValueError("invalid batch_rows")
    for start in range(0, rows, batch_rows):
        lines = []
        for index in range(start, min(rows, start + batch_rows)):
            key = 0 if index % 100 < hot_percent else 1 + (index % groups)
            lines.append(f"{index}\t{key}\n")
        stream.write("".join(lines))


def generate(output, rows, *, groups=1000, hot_percent=90, batch_rows=8192):
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise FileExistsError("refusing to overwrite fixture")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="ascii", newline="\n",
                                         dir=output.parent, delete=False) as stream:
            temporary = Path(stream.name)
            emit(stream, rows, groups=groups, hot_percent=hot_percent,
                 batch_rows=batch_rows)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temporary, output)
    finally:
        if temporary:
            temporary.unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--groups", type=int, default=1000)
    parser.add_argument("--hot-percent", type=int, default=90)
    parser.add_argument("--batch-rows", type=int, default=8192)
    args = parser.parse_args(argv)
    try:
        generate(args.output, args.rows, groups=args.groups,
                 hot_percent=args.hot_percent, batch_rows=args.batch_rows)
    except (OSError, ValueError) as error:
        parser.exit(1, f"skewed fixture failed: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

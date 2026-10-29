"""Produce a bijective UInt64 pseudorandom-looking permutation without a shuffle array.

Uses a full-period odd modular multiplier. This is deterministic fixture data,
NOT cryptographic randomness or a benchmark result.
"""
import argparse
import os
import tempfile
from pathlib import Path

MASK = (1 << 64) - 1
MULTIPLIER = 6364136223846793005


def permuted(index, seed=0):
    if type(index) is not int or type(seed) is not int or not 0 <= index <= MASK or not 0 <= seed <= MASK:
        raise ValueError("index and seed must be UInt64")
    return (index * MULTIPLIER + seed) & MASK


def emit(stream, rows, *, seed=0, batch_rows=8192):
    if type(rows) is not int or not 1 <= rows <= MASK + 1:
        raise ValueError("rows must fit in UInt64 domain")
    if type(seed) is not int or not 0 <= seed <= MASK:
        raise ValueError("seed must be UInt64")
    if type(batch_rows) is not int or not 1 <= batch_rows <= 1_000_000:
        raise ValueError("batch_rows must be in 1..1000000")
    value = seed
    for start in range(0, rows, batch_rows):
        # Modular addition is enough to preserve the exact affine UInt64 sequence.
        pending = []
        for _ in range(start, min(rows, start + batch_rows)):
            pending.append(f"{value}\n")
            value = (value + MULTIPLIER) & MASK
        stream.write("".join(pending))


def generate(output, rows, *, seed=0, batch_rows=8192):
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise FileExistsError(f"output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="ascii", newline="\n", dir=output.parent,
            prefix=f".{output.name}.", delete=False
        ) as stream:
            temporary = Path(stream.name)
            emit(stream, rows, seed=seed, batch_rows=batch_rows)
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
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--batch-rows", type=int, default=8192)
    args = parser.parse_args(argv)
    try:
        generate(args.output, args.rows, seed=args.seed, batch_rows=args.batch_rows)
    except (OSError, ValueError) as error:
        parser.exit(1, f"permuted fixture failed: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

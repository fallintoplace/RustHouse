"""Constant-space exact numeric references for zero-based UInt64 fixtures.

No RustHouse database is read. These oracles assume ids are 0..rows-1 exactly.
"""
import argparse
import json

MAX_UINT64 = (1 << 64) - 1


def sum_range(start, stop):
    if type(start) is not int or type(stop) is not int or not 0 <= start <= stop <= MAX_UINT64 + 1:
        raise ValueError("expected an ordered UInt64 half-open range")
    count = stop - start
    return count * (start + stop - 1) // 2 if count else 0


def modulo_reference(rows, modulus, residue):
    if type(rows) is not int or not 0 <= rows <= MAX_UINT64 + 1:
        raise ValueError("invalid UInt64 row count")
    if (type(modulus) is not int or type(residue) is not int
            or not 1 <= modulus <= MAX_UINT64 or not 0 <= residue < modulus):
        raise ValueError("invalid modulus or residue")
    count = 0 if residue >= rows else (rows - 1 - residue) // modulus + 1
    total = count * residue + modulus * count * (count - 1) // 2
    return {"count": count, "sum": total}


def modulo_interval_reference(start, stop, modulus, residue):
    """Exact count and sum of integers matching a residue in [start, stop)."""
    if (type(start) is not int or type(stop) is not int
            or not 0 <= start <= stop <= MAX_UINT64 + 1):
        raise ValueError("invalid UInt64 interval")
    before = modulo_reference(start, modulus, residue)
    after = modulo_reference(stop, modulus, residue)
    return {"count": after["count"] - before["count"],
            "sum": after["sum"] - before["sum"]}


def progression_reference(start, rows, step=1):
    """Exact COUNT and SUM for a bounded arithmetic progression."""
    if (type(start) is not int or type(rows) is not int or type(step) is not int
            or start < 0 or rows < 1 or step < 1
            or start > MAX_UINT64 or (rows - 1) > (MAX_UINT64 - start) // step):
        raise ValueError("invalid or overflowing UInt64 progression")
    return {"count": rows, "sum": rows * (2 * start + (rows - 1) * step) // 2}


def parity_progression(start, rows, step, parity):
    # This helper is also public to benchmark callers that do not go through
    # make_oracles(), so reject impossible progressions before emitting sums.
    progression_reference(start, rows, step)
    if type(parity) is not int or parity not in (0, 1):
        raise ValueError("parity must be 0 or 1")
    if step % 2 == 0:
        count = rows if start % 2 == parity else 0
        first = start
        spacing = step
    else:
        index = (parity - start) % 2
        count = (rows - index + 1) // 2 if index < rows else 0
        first = start + index * step
        spacing = 2 * step
    return {"count": count, "sum": count * (2 * first + (count - 1) * spacing) // 2
            if count else 0}


def make_oracles(rows, *, start=0, step=1):
    if type(rows) is not int or not 1 <= rows <= MAX_UINT64 + 1:
        raise ValueError("rows must be in 1..2^64")
    total = progression_reference(start, rows, step)
    even = parity_progression(start, rows, step, 0)
    odd = parity_progression(start, rows, step, 1)
    return {"rows": rows,
            "assumption": f"exactly one id in start={start}, step={step}, rows={rows}",
            "count_all": rows, "sum_all": total["sum"],
            "even": even, "odd": odd}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--step", type=int, default=1)
    parser.add_argument("--output")
    args = parser.parse_args(argv)
    try:
        content = json.dumps(make_oracles(args.rows, start=args.start, step=args.step), indent=2) + "\n"
    except ValueError as error:
        parser.exit(1, f"numeric oracle failed: {error}\n")
    if args.output:
        from pathlib import Path
        Path(args.output).write_text(content, encoding="utf-8")
    print(content, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

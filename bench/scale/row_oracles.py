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


def make_oracles(rows):
    if type(rows) is not int or not 1 <= rows <= MAX_UINT64 + 1:
        raise ValueError("rows must be in 1..2^64")
    even = modulo_reference(rows, 2, 0)
    odd = modulo_reference(rows, 2, 1)
    return {"rows": rows, "assumption": "exactly one id per integer in [0, rows)",
            "count_all": rows, "sum_all": sum_range(0, rows),
            "even": even, "odd": odd}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--output")
    args = parser.parse_args(argv)
    try:
        content = json.dumps(make_oracles(args.rows), indent=2) + "\n"
    except ValueError as error:
        parser.exit(1, f"numeric oracle failed: {error}\n")
    if args.output:
        from pathlib import Path
        Path(args.output).write_text(content, encoding="utf-8")
    print(content, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

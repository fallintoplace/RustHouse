#!/usr/bin/env python3
"""Verify TSV reference rows using exact text or arbitrary-precision numbers."""
import argparse
from decimal import Decimal, InvalidOperation
from pathlib import Path


def parse_tolerance(value):
    try:
        tolerance = Decimal(str(value))
    except InvalidOperation as error:
        raise ValueError("tolerance must be a decimal number") from error
    if not tolerance.is_finite() or tolerance < 0:
        raise ValueError("tolerance must be finite and nonnegative")
    return tolerance


def numeric_equal(left, right, tolerance=Decimal("0")):
    tolerance = parse_tolerance(tolerance)
    if left == right:
        try:
            return Decimal(left.decode("ascii")).is_finite()
        except (UnicodeDecodeError, InvalidOperation):
            return True
    try:
        lhs = Decimal(left.decode("ascii"))
        rhs = Decimal(right.decode("ascii"))
    except (UnicodeDecodeError, InvalidOperation):
        return False
    return lhs.is_finite() and rhs.is_finite() and abs(lhs - rhs) <= tolerance


def compare_tsv(actual, reference, tolerance=Decimal("0")):
    tolerance = parse_tolerance(tolerance)
    if actual.endswith(b"\n") != reference.endswith(b"\n"):
        return False
    left_rows = actual.splitlines()
    right_rows = reference.splitlines()
    if len(left_rows) != len(right_rows):
        return False
    for left, right in zip(left_rows, right_rows):
        a = left.split(b"\t")
        b = right.split(b"\t")
        if len(a) != len(b):
            return False
        if not all(numeric_equal(x, y, tolerance) for x, y in zip(a, b)):
            return False
    return True


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("actual", type=Path)
    parser.add_argument("reference", type=Path)
    parser.add_argument("--absolute-tolerance", default="0")
    args = parser.parse_args(argv)
    try:
        tolerance = parse_tolerance(args.absolute_tolerance)
        passed = compare_tsv(args.actual.read_bytes(), args.reference.read_bytes(), tolerance)
    except (OSError, ValueError) as error:
        parser.exit(2, f"comparison failed: {error}\n")
    print("passed" if passed else "mismatch")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())

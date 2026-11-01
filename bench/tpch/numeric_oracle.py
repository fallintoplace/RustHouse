#!/usr/bin/env python3
"""Verify TSV reference rows using exact text or arbitrary-precision numbers."""
import argparse
from decimal import (Decimal, DecimalException, InvalidOperation, localcontext,
                     ROUND_DOWN, ROUND_UP, MAX_EMAX, MIN_EMIN)
from pathlib import Path

MAX_INPUT_BYTES = 16 * 1024 * 1024


def read_input(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError("oracle input must be a regular, non-symlinked file")
    with path.open("rb") as source:
        payload = source.read(MAX_INPUT_BYTES + 1)
    if len(payload) > MAX_INPUT_BYTES:
        raise ValueError("numeric oracle input exceeds 16 MiB")
    return payload


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
    if not lhs.is_finite() or not rhs.is_finite():
        return False
    if tolerance == 0:
        # Exact decimal comparison avoids context rounding.
        return lhs == rhs
    # Directed rounding gives lower and upper bounds on the absolute gap.
    # Escalating precision avoids the default 28-digit context rounding a
    # near-threshold mismatch into a false positive. Extreme exponent gaps
    # do not require allocating enormous decimal coefficients.
    try:
        for precision in (28, 64, 128, 256, 512, 1024, 2048, 4096):
            with localcontext() as context:
                context.prec = precision
                context.Emax = MAX_EMAX
                context.Emin = MIN_EMIN
                context.rounding = ROUND_DOWN
                lower = abs(lhs - rhs)
                context.rounding = ROUND_UP
                upper = abs(lhs - rhs)
                if upper <= tolerance:
                    return True
                if lower > tolerance:
                    return False
        # Never affirm equality while the exact boundary remains ambiguous.
        return False
    except DecimalException:
        # Extreme decimal contexts must not crash the oracle.
        return False


def split_tsv_rows(data):
    normalized = data.replace(b"\r\n", b"\n")
    if b"\r" in normalized:
        return None
    if not data:
        return []
    rows = normalized.split(b"\n")
    if data.endswith(b"\n"):
        rows.pop()
    return rows


def compare_tsv(actual, reference, tolerance=Decimal("0")):
    tolerance = parse_tolerance(tolerance)
    if actual.endswith(b"\n") != reference.endswith(b"\n"):
        return False
    left_rows = split_tsv_rows(actual)
    right_rows = split_tsv_rows(reference)
    if left_rows is None or right_rows is None:
        return False
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
        passed = compare_tsv(read_input(args.actual), read_input(args.reference), tolerance)
    except (OSError, ValueError) as error:
        parser.exit(2, f"comparison failed: {error}\n")
    print("passed" if passed else "mismatch")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())

"""Compare RustHouse TSV rows against SQLite with precise numeric semantics.

Integer values must match exactly, including values beyond float64 precision.
Only SQLite floating results receive relative/absolute tolerance.
"""
import math
import sqlite3
from contextlib import closing


def cell_equal(actual, expected, rel_tol=1e-12, abs_tol=1e-12):
    if expected is None:
        return actual in ("NULL", r"\N")
    if isinstance(expected, bool):
        expected = int(expected)
    if isinstance(expected, int):
        try:
            return int(actual) == expected and actual.strip() == str(int(actual))
        except ValueError:
            return False
    if isinstance(expected, float):
        try:
            value = float(actual)
        except ValueError:
            return False
        return math.isfinite(value) and math.isfinite(expected) and math.isclose(
            value, expected, rel_tol=rel_tol, abs_tol=abs_tol
        )
    return actual == str(expected)


def parse_rows(output):
    if not isinstance(output, bytes):
        raise TypeError("RustHouse output must be TSV bytes")
    return [row.split("\t") for row in output.decode("utf-8").splitlines()]


def row_equal(actual, expected):
    return len(actual) == len(expected) and all(
        cell_equal(value, reference) for value, reference in zip(actual, expected)
    )


def assert_ordered_rows(actual, expected_rows):
    for number, (row, expected) in enumerate(zip(actual, expected_rows), 1):
        if not row_equal(row, expected):
            raise ValueError(f"row {number} differs from SQLite")


def assert_unordered_rows(actual, expected_rows):
    if len(actual) > 4096:
        raise ValueError("unordered comparison is limited to 4096 result rows")
    unmatched = list(actual)
    for expected in expected_rows:
        position = next((i for i, row in enumerate(unmatched) if row_equal(row, expected)), None)
        if position is None:
            raise ValueError("result multiset differs from SQLite")
        unmatched.pop(position)


def assert_matches(output, expected_rows, unordered=False):
    actual = parse_rows(output)
    expected_rows = list(expected_rows)
    if len(actual) != len(expected_rows):
        raise ValueError(f"row count mismatch: RustHouse={len(actual)}, SQLite={len(expected_rows)}")
    if unordered:
        assert_unordered_rows(actual, expected_rows)
    else:
        assert_ordered_rows(actual, expected_rows)


def sqlite_query(path, sql):
    with closing(sqlite3.connect(path)) as connection:
        return connection.execute(sql).fetchall()

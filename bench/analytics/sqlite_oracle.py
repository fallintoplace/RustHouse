"""Compare RustHouse TSV rows against SQLite with precise numeric semantics.

Integer values must match exactly, including values beyond float64 precision.
Only SQLite floating results receive relative/absolute tolerance.
"""
import math
import sqlite3
from contextlib import closing


def decode_tsv_value(raw):
    """Decode ClickHouse-style TSV escapes without conflating NULL and '\\N' text.

    Returns None only for the unescaped NULL marker. Input is UTF-8 TSV text;
    \\xHH escapes are bytes, not Unicode code points.
    """
    if raw == r"\N":
        return None
    if "\\" not in raw:
        return raw
    encoded = raw.encode("utf-8")
    output = bytearray()
    simple = {
        ord("t"): 9, ord("n"): 10, ord("r"): 13,
        ord("0"): 0, ord("b"): 8, ord("f"): 12,
        ord("v"): 11, ord("\\"): 92,
    }
    index = 0
    while index < len(encoded):
        char = encoded[index]
        if char != 92:
            output.append(char)
            index += 1
            continue
        index += 1
        if index >= len(encoded):
            raise ValueError("truncated TSV escape")
        char = encoded[index]
        if char == ord("x"):
            pair = encoded[index + 1:index + 3]
            if len(pair) != 2 or any(c not in b"0123456789abcdefABCDEF" for c in pair):
                raise ValueError("invalid TSV hex escape")
            output.append(int(pair, 16))
            index += 3
        elif char in simple:
            output.append(simple[char])
            index += 1
        else:
            raise ValueError("unsupported TSV escape")
    return output.decode("utf-8")


def cell_equal(actual, expected, rel_tol=1e-12, abs_tol=1e-12):
    if expected is None:
        return actual in ("NULL", r"\N")
    try:
        actual = decode_tsv_value(actual)
    except (UnicodeError, ValueError, AttributeError):
        return False
    if actual is None:
        return False
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
    decoded = output.decode("utf-8")
    if not decoded:
        return []
    if decoded.endswith("\n"):
        decoded = decoded[:-1]
    return [row.removesuffix("\r").split("\t") for row in decoded.split("\n")]


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
        rows = connection.execute(sql).fetchmany(100_001)
        if len(rows) > 100_000:
            raise ValueError("SQLite reference exceeds 100000 rows")
        return rows

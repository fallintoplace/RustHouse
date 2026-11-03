"""Bounded, value-free diagnostics for failed TPC-H reference comparisons."""
from collections import Counter


def describe_mismatch(actual, reference, mode):
    """Report shape and first differing position without exposing data values."""
    output = actual.splitlines()
    expected = reference.splitlines()
    if mode in ("exact", "numeric", "unordered-header"):
        if output[:1] != expected[:1]:
            return "output differs from reference: header mismatch"
        body_out, body_expected = output[1:], expected[1:]
    else:
        body_out, body_expected = output, expected
    if len(body_out) != len(body_expected):
        return ("output differs from reference: row count "
                f"expected {len(body_expected)}, got {len(body_out)}")
    if mode in ("unordered", "unordered-header"):
        output_counts, expected_counts = Counter(body_out), Counter(body_expected)
        missing = sum((expected_counts - output_counts).values())
        unexpected = sum((output_counts - expected_counts).values())
        return ("output differs from reference: unordered row multiset "
                f"has {missing} missing and {unexpected} unexpected rows")
    if mode == "numeric":
        return "output differs from numeric reference: cell value or shape mismatch"
    for row, (actual_row, expected_row) in enumerate(zip(body_out, body_expected), 1):
        if actual_row == expected_row:
            continue
        actual_fields = actual_row.split(b"\t")
        expected_fields = expected_row.split(b"\t")
        if len(actual_fields) != len(expected_fields):
            return (f"output differs from reference: row {row} has "
                    f"{len(actual_fields)} fields, expected {len(expected_fields)}")
        column = next((i for i, pair in enumerate(zip(actual_fields, expected_fields), 1)
                       if pair[0] != pair[1]), None)
        return f"output differs from reference: row {row}, column {column}"
    return "output differs from reference bytes (line endings or trailing newline)"

"""Exact, bounded-output SQL probes for frozen sequential-UInt64 wide parts.

This benchmarks stored data, never numbers(). The caller must independently
establish that id contains each integer in [0, rows) exactly once.
"""
import argparse
import json
import math
import re
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

from .row_oracles import modulo_reference, sum_range

CASE_DIR = Path(__file__).with_name("billion_cases")
MAX_ROWS = 4_000_000_000
MAX_OUTPUT_BYTES = 64 * 1024
CASE_KINDS = {
    "count_range", "sum_range", "min_range", "max_range",
    "count_between", "sum_between", "count_mod", "sum_mod",
    "count_in", "sum_in", "count_or", "sum_or", "count_ne",
    "topk_asc", "topk_desc",
}
CASE_FIELDS = {
    "count_range": ("lo", "hi"), "sum_range": ("lo", "hi"),
    "min_range": ("lo", "hi"), "max_range": ("lo", "hi"),
    "count_between": ("lo", "hi"), "sum_between": ("lo", "hi"),
    "count_mod": ("modulus", "residue"), "sum_mod": ("modulus", "residue"),
    "count_in": ("values",), "sum_in": ("values",),
    "count_or": ("lo", "hi"), "sum_or": ("lo", "hi"),
    "count_ne": ("value",),
    "topk_asc": ("lo", "hi", "limit", "offset"),
    "topk_desc": ("lo", "hi", "limit", "offset"),
}


def bound(name, rows):
    if type(name) is not str:
        raise ValueError("case boundary must be a named token")
    tokens = {
        "start": 0, "one": 1, "last": rows - 1, "end": rows,
        "p01": rows // 100, "p10": rows // 10, "p25": rows // 4,
        "p50": rows // 2, "p75": rows * 3 // 4,
        "p90": rows * 9 // 10, "p99": rows * 99 // 100,
        "first1024": min(rows, 1024),
        "first1m": min(rows, 1_000_000),
        "tail1m": max(0, rows - 1_000_000),
    }
    if name not in tokens:
        raise ValueError(f"unknown boundary token: {name}")
    return tokens[name]


def load_cases(directory=CASE_DIR):
    """Load only checked-in case files, rejecting ambiguous or edited schemas."""
    cases = []
    seen = set()
    for path in sorted(Path(directory).glob("*.json")):
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"unsafe case file: {path}")
        case = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(case, dict):
            raise ValueError(f"invalid case: {path}")
        name, kind = case.get("id"), case.get("kind")
        if type(name) is not str or not re.fullmatch(r"[a-z][a-z0-9-]{2,63}", name):
            raise ValueError(f"invalid case id: {path}")
        if name != path.stem or name in seen:
            raise ValueError(f"duplicate or mismatched case id: {path}")
        if kind not in CASE_KINDS or set(case) != {"id", "kind", *CASE_FIELDS[kind]}:
            raise ValueError(f"invalid case schema: {path}")
        seen.add(name)
        cases.append(case)
    if not cases:
        raise ValueError("no billion-row query cases found")
    return cases


def compile_case(case, rows):
    """Build SQL and a separate exact numeric oracle without evaluating SQL."""
    if type(rows) is not int or not 1 <= rows <= MAX_ROWS:
        raise ValueError("rows must be in 1..4000000000")
    kind = case["kind"]
    if kind not in CASE_KINDS or set(case) != {"id", "kind", *CASE_FIELDS[kind]}:
        raise ValueError("invalid case schema")
    if kind.endswith("_range") or kind in {
        "count_between", "sum_between", "count_or", "sum_or",
        "topk_asc", "topk_desc",
    }:
        lo, hi = bound(case["lo"], rows), bound(case["hi"], rows)
        if not 0 <= lo <= hi <= rows or (kind not in ("count_or", "sum_or") and lo == hi):
            raise ValueError("invalid range boundaries")
    if kind in ("count_range", "sum_range", "min_range", "max_range"):
        field = {"count_range": "count(*)", "sum_range": "sum(id)",
                 "min_range": "min(id)", "max_range": "max(id)"}[kind]
        sql = f"SELECT {field} FROM part WHERE id >= {lo} AND id < {hi}"
        expected = {"count_range": hi - lo, "sum_range": sum_range(lo, hi),
                    "min_range": lo, "max_range": hi - 1}[kind]
    elif kind in ("count_between", "sum_between"):
        if hi >= rows:
            raise ValueError("BETWEEN upper bound must be an existing id")
        field = "count(*)" if kind == "count_between" else "sum(id)"
        sql = f"SELECT {field} FROM part WHERE id BETWEEN {lo} AND {hi}"
        expected = hi - lo + 1 if kind == "count_between" else sum_range(lo, hi + 1)
    elif kind in ("count_mod", "sum_mod"):
        modulus, residue = case["modulus"], case["residue"]
        if (type(modulus) is not int or type(residue) is not int
                or not 1 <= modulus <= 65536 or not 0 <= residue < modulus):
            raise ValueError("invalid modulo predicate")
        field = "count(*)" if kind == "count_mod" else "sum(id)"
        sql = f"SELECT {field} FROM part WHERE id % {modulus} = {residue}"
        reference = modulo_reference(rows, modulus, residue)
        expected = reference["count" if kind == "count_mod" else "sum"]
    elif kind in ("count_in", "sum_in"):
        tokens = case["values"]
        if not isinstance(tokens, list) or not 1 <= len(tokens) <= 16:
            raise ValueError("IN list needs 1..16 named ids")
        ids = [bound(token, rows) for token in tokens]
        field = "count(*)" if kind == "count_in" else "sum(id)"
        sql = f"SELECT {field} FROM part WHERE id IN ({', '.join(map(str, ids))})"
        present = {value for value in ids if value < rows}
        expected = len(present) if kind == "count_in" else sum(present)
    elif kind in ("count_or", "sum_or"):
        field = "count(*)" if kind == "count_or" else "sum(id)"
        sql = f"SELECT {field} FROM part WHERE id < {lo} OR id >= {hi}"
        expected = lo + rows - hi if kind == "count_or" else (
            sum_range(0, lo) + sum_range(hi, rows)
        )
    elif kind == "count_ne":
        value = bound(case["value"], rows)
        sql = f"SELECT count(*) FROM part WHERE id != {value}"
        expected = rows - (value < rows)
    elif kind in ("topk_asc", "topk_desc"):
        limit, offset = case["limit"], case["offset"]
        if (type(limit) is not int or type(offset) is not int
                or not 1 <= limit <= 128 or not 0 <= offset <= 1_000_000):
            raise ValueError("TOP-K limit or offset is out of bounds")
        direction = "ASC" if kind == "topk_asc" else "DESC"
        sql = (f"SELECT id FROM part WHERE id >= {lo} AND id < {hi} "
               f"ORDER BY id {direction} LIMIT {limit} OFFSET {offset}")
        size = max(0, min(limit, hi - lo - offset))
        if kind == "topk_asc":
            values = range(lo + offset, lo + offset + size)
        else:
            values = range(hi - 1 - offset, hi - 1 - offset - size, -1)
        return sql, "".join(f"{value}\n" for value in values).encode("ascii")
    else:
        raise ValueError("unsupported case kind")
    return sql, f"{expected}\n".encode("ascii")


def invoke(binary, part, sql, timeout):
    """Keep result bytes off the heap and reject any unexpected large output."""
    command = [str(binary), "sql", sql, "--part", str(part),
               "--format", "tsv-no-header", "--max-rows", "128"]
    started = time.perf_counter()
    with tempfile.TemporaryFile() as output:
        result = subprocess.run(command, stdout=output, stderr=subprocess.PIPE,
                                check=False, timeout=timeout)
        seconds = time.perf_counter() - started
        if result.returncode:
            raise RuntimeError(
                f"SQL failed ({result.returncode}): "
                f"{result.stderr.decode('utf-8', 'replace')[-500:]}"
            )
        output.seek(0)
        data = output.read(MAX_OUTPUT_BYTES + 1)
        if len(data) > MAX_OUTPUT_BYTES:
            raise ValueError("query output exceeds 64 KiB")
    return data, seconds


def benchmark(binary, part, rows, *, names=(), runs=3, warmups=1, timeout=600,
              execute=None, cases=None):
    if type(rows) is not int or not 1 <= rows <= MAX_ROWS:
        raise ValueError("rows must be in 1..4000000000")
    if (type(runs) is not int or type(warmups) is not int
            or runs < 1 or warmups < 0 or type(timeout) not in (int, float)
            or not math.isfinite(timeout) or timeout <= 0):
        raise ValueError("invalid run counts or timeout")
    cases = load_cases() if cases is None else cases
    available = {case["id"]: case for case in cases}
    if len(available) != len(cases):
        raise ValueError("duplicate case IDs")
    if names:
        if any(name not in available for name in names):
            raise ValueError("unknown query case")
        cases = [available[name] for name in dict.fromkeys(names)]
    run = execute or invoke
    actual, _ = run(binary, part, "SELECT count(*) FROM part", timeout)
    if actual != f"{rows}\n".encode("ascii"):
        raise ValueError("snapshot row count differs from the expected rows")
    results = []
    for case in cases:
        sql, expected = compile_case(case, rows)
        samples = []
        for iteration in range(warmups + runs):
            data, seconds = run(binary, part, sql, timeout)
            if data != expected:
                raise ValueError(f"{case['id']}: wrong physical SQL result")
            if type(seconds) not in (int, float) or not math.isfinite(seconds) or seconds < 0:
                raise ValueError(f"{case['id']}: invalid query duration")
            if iteration >= warmups:
                samples.append(seconds)
        ordered = sorted(samples)
        results.append({
            "id": case["id"], "kind": case["kind"], "sql": sql,
            "result_bytes": len(expected), "verified": True,
            "median_seconds": statistics.median(samples),
            "p90_seconds": ordered[math.ceil(0.9 * len(ordered)) - 1],
            "samples_seconds": samples,
        })
    return {
        "workload": "rusthouse-billion-physical-query-matrix",
        "rows_claimed": rows, "snapshot_count_matched": True,
        "data_assumption": "frozen wide-part id is exactly 0..rows-1 once each",
        "scope": "physical SQL output, fresh CLI process, potentially warm OS cache",
        "runs": runs, "warmups": warmups, "all_verified": True,
        "results": results,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--part", required=True, type=Path)
    parser.add_argument("--rows", required=True, type=int)
    parser.add_argument("--case", action="append", default=[], help="Select case ID, repeatable")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--require-billion-rows", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.require_billion_rows and args.rows < 1_000_000_000:
        parser.error("--require-billion-rows requires at least 1000000000 rows")
    try:
        report = benchmark(args.binary, args.part, args.rows, names=args.case,
                           runs=args.runs, warmups=args.warmups, timeout=args.timeout)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired,
            json.JSONDecodeError) as error:
        parser.exit(1, f"billion-row matrix failed: {error}\n")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

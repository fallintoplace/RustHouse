"""Verify selective SQL over a known persisted sequential UInt64 fact table.

The database must contain events(id UInt64) with each value in [0, rows)
exactly once. RustHouse doctor validates the persisted snapshot's structural
integrity and row count, but cannot independently prove sequence completeness.
Unlike numbers(), these queries read a physical table. No official performance
claim can be made without the full report and machine/dataset description.
"""
import argparse
import json
import math
import os
import platform
import re
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

from bench.scale.row_oracles import modulo_reference, sum_range

MAX_ROWS = 4_000_000_000  # SUM(id) stays within UInt64 for this range.
CASE_DIRECTORY = Path(__file__).with_name("billion_matrix")
IDENTIFIER = re.compile(r"[a-z][a-z0-9-]{0,63}\Z")


def _integer(value, name, minimum=0, maximum=MAX_ROWS):
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError(f"{name} must be an integer in {minimum}..{maximum}")
    return value


def _interval(case, rows):
    lower = _integer(case.get("lower_ppm"), "lower_ppm", 0, 1_000_000)
    upper = _integer(case.get("upper_ppm"), "upper_ppm", 0, 1_000_000)
    if lower >= upper:
        raise ValueError("range must have lower_ppm < upper_ppm")
    start, stop = rows * lower // 1_000_000, rows * upper // 1_000_000
    if start >= stop:
        raise ValueError("range is empty at this row count")
    return start, stop


def _modulo(case):
    modulus = _integer(case.get("modulus"), "modulus", 1, 1_000_000)
    residue = _integer(case.get("residue"), "residue", 0, modulus - 1)
    return modulus, residue


def _mod_interval(start, stop, modulus, residue):
    high = modulo_reference(stop, modulus, residue)
    low = modulo_reference(start, modulus, residue)
    return high["count"] - low["count"], high["sum"] - low["sum"]


def compile_case(case, rows):
    """Turn a data-only case into SQL plus exact independent expected TSV bytes."""
    _integer(rows, "rows", 100_000, MAX_ROWS)
    if not isinstance(case, dict) or not isinstance(case.get("name"), str) or not IDENTIFIER.fullmatch(case["name"]):
        raise ValueError("case requires a lowercase hyphenated name")
    kind = case.get("kind")
    if kind == "page":
        direction = case.get("direction")
        if direction not in ("ASC", "DESC"):
            raise ValueError("page direction must be ASC or DESC")
        limit = _integer(case.get("limit"), "limit", 1, 64)
        offset = _integer(case.get("offset"), "offset", 0, 8192)
        has_modulo = "modulus" in case or "residue" in case
        modulus, residue = _modulo(case) if has_modulo else (1, 0)
        available, _ = _mod_interval(0, rows, modulus, residue)
        if offset + limit > available:
            raise ValueError("page would be empty or incomplete")
        positions = (range(offset, offset + limit) if direction == "ASC"
                     else range(available - 1 - offset, available - 1 - offset - limit, -1))
        expected = "".join(f"{residue + modulus * index}\n" for index in positions).encode("ascii")
        condition = f" WHERE id % {modulus} = {residue}" if has_modulo else ""
        sql = (f"SELECT id FROM events{condition} ORDER BY id {direction}"
               f" LIMIT {limit} OFFSET {offset}")
        return sql, expected, available

    aggregate = case.get("aggregate")
    if aggregate not in ("count", "sum"):
        raise ValueError("aggregate must be count or sum")
    expression = "count(*)" if aggregate == "count" else "sum(id)"
    if kind == "modulo":
        modulus, residue = _modulo(case)
        matched, total = _mod_interval(0, rows, modulus, residue)
        condition = f"id % {modulus} = {residue}"
    elif kind == "range":
        start, stop = _interval(case, rows)
        matched, total = stop - start, sum_range(start, stop)
        condition = f"id >= {start} AND id < {stop}"
    elif kind == "range-modulo":
        start, stop = _interval(case, rows)
        modulus, residue = _modulo(case)
        matched, total = _mod_interval(start, stop, modulus, residue)
        condition = (f"id >= {start} AND id < {stop}"
                     f" AND id % {modulus} = {residue}")
    elif kind == "membership":
        values = case.get("ids")
        if (not isinstance(values, list) or not 1 <= len(values) <= 16
                or any(type(value) is not int or not 0 <= value < MAX_ROWS for value in values)
                or len(set(values)) != len(values)):
            raise ValueError("ids must be 1..16 distinct UInt64 values")
        exclude = case.get("exclude", False)
        if type(exclude) is not bool:
            raise ValueError("exclude must be a boolean")
        included = [value for value in values if value < rows]
        matched, total = len(included), sum(included)
        if exclude:
            matched, total = rows - matched, sum_range(0, rows) - total
        condition = f"id {'NOT IN' if exclude else 'IN'} ({', '.join(map(str, values))})"
    else:
        raise ValueError(f"unsupported kind: {kind!r}")
    if aggregate == "sum" and matched == 0:
        raise ValueError("SUM over an empty selection produces NULL, not zero")
    sql = f"SELECT {expression} FROM events WHERE {condition}"
    return sql, f"{matched if aggregate == 'count' else total}\n".encode("ascii"), matched


def _invoke(command, *, timeout, execute=None):
    run = execute or subprocess.run
    started = time.perf_counter()
    result = run(command, capture_output=True, check=False, timeout=timeout)
    elapsed = time.perf_counter() - started
    if result.returncode:
        raise RuntimeError(f"command failed with exit {result.returncode}: {result.stderr[-600:]!r}")
    return result.stdout, elapsed


def verify_database(binary, database, rows, *, timeout=600, execute=None):
    output, _ = _invoke([str(binary), "doctor", "--database", str(database)],
                        timeout=timeout, execute=execute)
    if not re.search(rb"(?m)^OK events: " + str(rows).encode("ascii") + rb" rows, ", output):
        raise ValueError("doctor did not verify the expected events row count")
    return True


def execute_case(binary, database, case, rows, *, runs=3, warmups=1,
                 timeout=600, execute=None, max_p90_seconds=None):
    sql, expected, matched = compile_case(case, rows)
    _integer(runs, "runs", 1, 100)
    _integer(warmups, "warmups", 0, 100)
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    if max_p90_seconds is not None and (type(max_p90_seconds) not in (int, float)
                                        or not math.isfinite(max_p90_seconds)
                                        or max_p90_seconds <= 0):
        raise ValueError("max_p90_seconds must be finite and positive")
    samples = []
    for index in range(runs + warmups):
        output, elapsed = _invoke(
            [str(binary), "sql", sql, "--database", str(database),
             "--format", "tsv-no-header"], timeout=timeout, execute=execute)
        if output != expected:
            raise ValueError(f"{case['name']}: wrong output on sample {index + 1}: {output[:120]!r}")
        if index >= warmups:
            samples.append(elapsed)
    p90 = sorted(samples)[math.ceil(len(samples) * .9) - 1]
    return {"name": case["name"], "kind": case["kind"], "sql": sql,
            "matched_rows_expected": matched, "verified": True,
            "runs": runs, "warmups": warmups,
            "median_seconds": statistics.median(samples), "p90_seconds": p90,
            "samples_seconds": samples,
            "within_target": max_p90_seconds is None or p90 <= max_p90_seconds}


def load_cases(directory):
    paths = sorted(Path(directory).glob("[0-9][0-9]-*.json"))
    if not paths:
        raise ValueError("no numbered cases found")
    names = set()
    cases = []
    for path in paths:
        case = json.loads(path.read_text(encoding="utf-8"))
        if (not isinstance(case, dict) or not isinstance(case.get("name"), str)
                or case["name"] in names):
            raise ValueError(f"duplicate or malformed case: {path}")
        names.add(case["name"])
        cases.append(case)
    return cases


def benchmark(binary, database, rows, directory=CASE_DIRECTORY, *, runs=3,
              warmups=1, timeout=600, max_p90_seconds=None, execute=None):
    _integer(rows, "rows", 100_000, MAX_ROWS)
    _integer(runs, "runs", 1, 100)
    _integer(warmups, "warmups", 0, 100)
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    if max_p90_seconds is not None and (
            type(max_p90_seconds) not in (int, float)
            or not math.isfinite(max_p90_seconds) or max_p90_seconds <= 0):
        raise ValueError("max_p90_seconds must be finite and positive")
    cases = load_cases(directory)
    # Fail on malformed cases before contacting the database or spending hours scanning.
    for case in cases:
        compile_case(case, rows)
    verify_database(binary, database, rows, timeout=timeout, execute=execute)
    results = [execute_case(binary, database, case, rows, runs=runs, warmups=warmups,
                            timeout=timeout, execute=execute,
                            max_p90_seconds=max_p90_seconds) for case in cases]
    return {"workload": "persisted-billion-row-query-matrix",
            "rows_written_and_doctor_verified": rows,
            "case_count": len(cases), "verified": all(r["verified"] for r in results),
            "all_within_target": all(r["within_target"] for r in results),
            "assumption": "events.id is exactly once each integer in [0, rows); doctor cannot prove completeness",
            "source": "existing physical RustHouse database, not numbers()",
            "official_benchmark": False, "platform": platform.platform(),
            "cpu_count": os.cpu_count(), "revision": os.environ.get("GITHUB_SHA"),
            "results": results}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--cases-dir", type=Path, default=CASE_DIRECTORY)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--max-p90-seconds", type=float)
    parser.add_argument("--require-billion-rows", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.require_billion_rows and args.rows < 1_000_000_000:
        parser.error("--require-billion-rows requires at least 1,000,000,000 rows")
    try:
        report = benchmark(args.binary, args.database, args.rows, args.cases_dir,
                           runs=args.runs, warmups=args.warmups,
                           timeout=args.timeout, max_p90_seconds=args.max_p90_seconds)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            # Only publish complete, fully verified reports.
            with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8",
                    dir=args.output.parent, prefix=".billion-matrix-", delete=False) as stream:
                temporary = Path(stream.name)
                stream.write(rendered)
            try:
                os.replace(temporary, args.output)
            finally:
                temporary.unlink(missing_ok=True)
        print(rendered, end="")
        return 0 if report["all_within_target"] else 1
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired,
            json.JSONDecodeError) as error:
        parser.exit(1, f"billion-row query matrix failed: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())

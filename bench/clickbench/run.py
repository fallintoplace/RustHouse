#!/usr/bin/env python3
"""Run ClickBench SQL with honest coverage for projected input tables."""

import argparse
from decimal import Decimal, InvalidOperation
import json
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, latency_summary

INTEGER = re.compile(r"[+-]?[0-9]+\Z")
RELATIVE_TOLERANCE = Decimal("1e-12")
OFFICIAL_QUERY_COUNT = 43
SUCCESS = {"passed", "unverified"}
SKIPPED = {"skipped_missing_columns", "unknown_requirements"}
MAX_REFERENCE_BYTES = 16 * 1024 * 1024


def load_queries(path):
    """ClickBench's queries.sql has one statement per non-comment line."""
    queries = []
    for line in path.read_text(encoding="utf-8").splitlines():
        statement = line.strip()
        if not statement or statement.startswith("--") or statement.startswith("#"):
            continue
        statement = statement.removesuffix(";").strip()
        if not statement:
            continue
        if (not re.match(r"(?i)^SELECT\s", statement) or ";" in statement
                or len(statement.encode("utf-8")) > 65_536):
            raise ValueError("ClickBench workload requires one bounded SELECT per line")
        queries.append((len(queries) + 1, statement))
    if not queries:
        raise ValueError("no ClickBench SQL statements found")
    return queries


def select_queries(queries, selected):
    if not selected:
        return queries
    try:
        identifiers = {int(number) for number in selected.split(",")}
    except ValueError as error:
        raise ValueError("--only requires comma-separated query numbers") from error
    if not identifiers or not identifiers.issubset({number for number, _ in queries}):
        raise ValueError("--only contains an unavailable query number")
    return [(number, sql) for number, sql in queries if number in identifiers]


def load_projection(path):
    """Never guess required columns by tokenizing SQL expressions."""
    if path is None:
        return None
    document = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise ValueError("projection manifest must be a JSON object")
    available = document.get("available_columns")
    required = document.get("query_columns")
    if not isinstance(available, list) or not isinstance(required, dict):
        raise ValueError("projection requires available_columns list and query_columns object")
    if any(not isinstance(name, str) or not name for name in available):
        raise ValueError("available_columns must contain nonempty names")
    for number, columns in required.items():
        if not isinstance(number, str) or not number.isdigit() or int(number) < 1:
            raise ValueError("query_columns keys must be positive query numbers")
        if not isinstance(columns, list) or any(
            not isinstance(name, str) or not name for name in columns
        ):
            raise ValueError("each query_columns entry must contain a list of column names")
    return {"available": set(available), "required": required}


def preflight(number, projection):
    if projection is None:
        return None, []
    required = projection["required"].get(str(number))
    if required is None:
        return "unknown_requirements", []
    missing = sorted(set(required) - projection["available"])
    if missing:
        return "skipped_missing_columns", missing
    return None, []


def matches_reference(actual, reference):
    """Match integer cells exactly and fractional cells to relative 1e-12."""
    if actual == reference:
        return True
    try:
        current_rows = actual.decode("utf-8").splitlines()
        expected_rows = reference.decode("utf-8").splitlines()
    except UnicodeDecodeError:
        return False
    if len(current_rows) != len(expected_rows):
        return False
    for current, expected in zip(current_rows, expected_rows):
        current_cells = current.split("\t")
        expected_cells = expected.split("\t")
        if len(current_cells) != len(expected_cells):
            return False
        for value, target in zip(current_cells, expected_cells):
            if value == target:
                continue
            if INTEGER.fullmatch(value) and INTEGER.fullmatch(target):
                if int(value) != int(target):
                    return False
                continue
            try:
                actual_number = Decimal(value)
                expected_number = Decimal(target)
            except InvalidOperation:
                return False
            if not actual_number.is_finite() or not expected_number.is_finite():
                return False
            if abs(actual_number - expected_number) > (
                abs(expected_number) * RELATIVE_TOLERANCE
            ):
                return False
    return True


def error_status(message):
    lowered = message.lower()
    if "sql parse error" in lowered or "parse error" in lowered:
        return "parse_error"
    if "unknown column" in lowered or "column not found" in lowered:
        return "schema_error"
    return "execution_error"


def benchmark_one(number, sql, execute, runs, warmups, reference=None):
    status = "passed" if reference is not None else "unverified"
    elapsed = []
    error = None
    for iteration in range(-warmups, runs):
        start = time.perf_counter() if iteration >= 0 else None
        try:
            output = execute(sql)
        except subprocess.TimeoutExpired:
            status, error = "timeout", "SQL execution timed out"
            break
        except (OSError, RuntimeError, ValueError) as failure:
            error = str(failure)
            status = error_status(error)
            break
        # Measure the engine invocation, not Python golden-output comparison.
        finished = time.perf_counter() if start is not None else None
        if reference is not None and not matches_reference(output, reference):
            status, error = "mismatch", "output differs from the reference"
            break
        if finished is not None:
            elapsed.append(finished - start)
    if status not in SUCCESS:
        elapsed = []
    return {
        "query": number,
        "sql": sql,
        "status": status,
        "timings_seconds": elapsed,
        "latency": latency_summary(elapsed),
        "error": error,
    }


def coverage(results):
    total = len(results)
    executed = sum(result["status"] in SUCCESS for result in results)
    skipped_columns = sum(result["status"] == "skipped_missing_columns" for result in results)
    unknown = sum(result["status"] == "unknown_requirements" for result in results)
    eligible = total - skipped_columns - unknown
    verified = sum(result["status"] == "passed" for result in results)
    return {
        "total_queries": total,
        "eligible_queries": eligible,
        "executed_queries": executed,
        "verified_queries": verified,
        "skipped_missing_columns": skipped_columns,
        "unknown_requirements": unknown,
        "failed_eligible_queries": eligible - executed,
        "execution_success_percent_of_eligible": (
            round(executed * 100 / eligible, 2) if eligible else 0.0
        ),
        "verified_percent_of_total": round(verified * 100 / total, 2) if total else 0.0,
    }


def run(queries, execute, runs, warmups, projection=None, references=None):
    results = []
    for number, sql in queries:
        skip, missing = preflight(number, projection)
        if skip is not None:
            results.append({
                "query": number, "sql": sql, "status": skip,
                "missing_columns": missing, "timings_seconds": [], "latency": None,
                "error": None,
            })
            continue
        reference = None
        if references is not None:
            path = references / f"q{number:02}.tsv"
            if not path.is_file():
                results.append({
                    "query": number, "sql": sql, "status": "missing_reference",
                    "missing_columns": [], "timings_seconds": [],
                    "latency": None, "error": str(path),
                })
                continue
            with path.open("rb") as source:
                reference = source.read(MAX_REFERENCE_BYTES + 1)
            if len(reference) > MAX_REFERENCE_BYTES:
                results.append({
                    "query": number, "sql": sql, "status": "reference_too_large",
                    "missing_columns": [], "timings_seconds": [],
                    "latency": None, "error": str(path),
                })
                continue
        result = benchmark_one(number, sql, execute, runs, warmups, reference)
        result["missing_columns"] = []
        results.append(result)
    return results


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--queries-file", type=Path, required=True)
    parser.add_argument("--projection-manifest", type=Path)
    parser.add_argument("--reference-dir", type=Path)
    parser.add_argument("--only", help="Comma-separated original ClickBench query numbers")
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    options = parser.parse_args(argv)
    if options.runs < 1 or options.warmups < 0 or options.timeout <= 0:
        parser.error("runs and timeout must be positive; warmups cannot be negative")
    try:
        all_queries = load_queries(options.queries_file)
        queries = select_queries(all_queries, options.only)
        projection = load_projection(options.projection_manifest)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    execute = lambda sql: cli_sql(options.binary, options.database, sql, options.timeout)
    results = run(
        queries, execute, options.runs, options.warmups, projection, options.reference_dir
    )
    report = {
        "engine": "rusthouse-cli",
        "workload": "ClickBench SQL",
        "official_query_count": OFFICIAL_QUERY_COUNT,
        "query_file_count": len(all_queries),
        "complete_official_query_list": len(all_queries) == OFFICIAL_QUERY_COUNT,
        "timing_scope": "warm-cache end-to-end CLI (startup and filesystem reads included)",
        "projection_manifest": str(options.projection_manifest) if projection else None,
        "reference_checked": options.reference_dir is not None,
        "runs": options.runs,
        "warmups": options.warmups,
        "coverage": coverage(results),
        "results": results,
    }
    if options.output is not None:
        options.output.parent.mkdir(parents=True, exist_ok=True)
    emit_json(report, options.output)
    return int(
        not any(result["status"] in SUCCESS for result in results)
        or any(result["status"] not in SUCCESS | SKIPPED for result in results)
    )


if __name__ == "__main__":
    raise SystemExit(main())

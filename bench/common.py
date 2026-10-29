"""Shared SQL invocation, timing statistics, and JSON output for workload runners."""

import json
import math
import statistics
import subprocess
import sys
from pathlib import Path


def cli_sql(binary, database, statement, timeout, *, select_format="tsv-no-header"):
    """Execute SQL via the CLI and return exact output bytes for verification."""
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    # --format is a SELECT-only option. DDL, COPY, and INSERT reject it.
    command = [str(binary), "sql", statement, "--database", str(database)]
    keyword = statement.split(maxsplit=1)[0].upper() if statement.strip() else ""
    if keyword == "SELECT":
        if select_format not in ("tsv-no-header", "jsonl"):
            raise ValueError("unsupported verification SELECT format")
        command.extend(["--format", select_format])
    elif select_format != "tsv-no-header":
        raise ValueError("a verification format can be used only with SELECT")
    completed = subprocess.run(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        timeout=timeout,
    )
    if completed.returncode:
        error = completed.stderr.decode("utf-8", "replace")[:2000]
        raise RuntimeError(f"exit {completed.returncode}: {error}")
    return completed.stdout


def latency_summary(seconds):
    """Report a common nearest-rank p90 for wall-clock timings."""
    if not seconds:
        return None
    for value in seconds:
        if type(value) not in (int, float) or value < 0:
            raise ValueError("timing samples must be finite nonnegative numbers")
        try:
            finite = math.isfinite(value)
        except OverflowError:
            finite = False
        if not finite:
            raise ValueError("timing samples must be finite nonnegative numbers")
    ordered = sorted(seconds)
    return {
        "min_seconds": ordered[0],
        "median_seconds": statistics.median(ordered),
        "p90_seconds": ordered[math.ceil(0.9 * len(ordered)) - 1],
        "max_seconds": ordered[-1],
    }


def emit_json(report, path=None):
    """Write a report to stdout and, if requested, an identical JSON file."""
    rendered = json.dumps(report, indent=2) + "\n"
    if path is not None:
        target = Path(path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(rendered, encoding="utf-8")
    sys.stdout.write(rendered)


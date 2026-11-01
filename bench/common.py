"""Shared SQL invocation, timing statistics, and JSON output for workload runners."""

import json
import math
import os
import tempfile
import subprocess
import sys
from pathlib import Path

def file_change_time_ns(metadata):
    """Compare path and handle metadata consistently on Windows and POSIX."""
    # Windows Python 3.12 may report different st_ctime_ns on the same open file.
    # Birth time remains stable across path lstat and descriptor fstat.
    if os.name == "nt":
        return getattr(metadata, "st_birthtime_ns", metadata.st_ctime_ns)
    return metadata.st_ctime_ns


MAX_SQL_STDOUT_BYTES = 64 * 1024 * 1024


def _leading_sql_keyword(statement):
    """Identify SELECT after leading comments without changing the SQL text."""
    remainder = statement.lstrip()
    while remainder:
        if remainder.startswith(("--", "#")):
            end = remainder.find("\n")
            if end < 0:
                return ""
            remainder = remainder[end + 1:].lstrip()
        elif remainder.startswith("/*"):
            end = remainder.find("*/", 2)
            if end < 0:
                return ""
            remainder = remainder[end + 2:].lstrip()
        else:
            break
    return remainder.split(maxsplit=1)[0].upper() if remainder else ""


def cli_sql(binary, database, statement, timeout, *, select_format="tsv-no-header"):
    """Execute SQL via the CLI and return exact output bytes for verification."""
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    # --format is a SELECT-only option. DDL, COPY, and INSERT reject it.
    command = [str(binary), "sql", statement, "--database", str(database)]
    keyword = _leading_sql_keyword(statement)
    if keyword == "SELECT":
        if select_format not in ("tsv-no-header", "jsonl"):
            raise ValueError("unsupported verification SELECT format")
        command.extend(["--format", select_format])
    elif select_format != "tsv-no-header":
        raise ValueError("a verification format can be used only with SELECT")
    # Do not buffer an untrusted query result in subprocess.PIPE. Only a
    # bounded prefix is ever read back into the Python heap.
    extra = {}
    if os.name == "posix":
        import resource

        def limit_child_output():
            _, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
            cap = MAX_SQL_STDOUT_BYTES + 1
            if hard != resource.RLIM_INFINITY:
                cap = min(cap, hard)
            resource.setrlimit(resource.RLIMIT_FSIZE, (cap, hard))

        extra["preexec_fn"] = limit_child_output

    with tempfile.TemporaryFile(mode="w+b") as output, tempfile.TemporaryFile(mode="w+b") as errors:
        completed = subprocess.run(
            command,
            stdout=output,
            stderr=errors,
            check=False,
            timeout=timeout,
            **extra,
        )
        if completed.returncode:
            output.seek(0, os.SEEK_END)
            if output.tell() > MAX_SQL_STDOUT_BYTES:
                raise ValueError("SQL query output exceeds 64 MiB")
            errors.seek(0)
            error = errors.read(2000).decode("utf-8", "replace")
            raise RuntimeError(f"exit {completed.returncode}: {error}")
        output.seek(0)
        result = output.read(MAX_SQL_STDOUT_BYTES + 1)
        if len(result) > MAX_SQL_STDOUT_BYTES:
            raise ValueError("SQL query output exceeds 64 MiB")
        return result


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
    middle = len(ordered) // 2
    median = (ordered[middle] if len(ordered) % 2
              else ordered[middle - 1] + (ordered[middle] - ordered[middle - 1]) / 2)
    return {
        "min_seconds": ordered[0],
        "median_seconds": median,
        "p90_seconds": ordered[math.ceil(0.9 * len(ordered)) - 1],
        "max_seconds": ordered[-1],
    }


def emit_json(report, path=None):
    """Write a report to stdout and, if requested, an identical JSON file."""
    rendered = json.dumps(report, indent=2, allow_nan=False) + "\n"
    if path is not None:
        target = Path(path)
        target.parent.mkdir(parents=True, exist_ok=True)
        pending = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w", encoding="utf-8", dir=target.parent,
                prefix=f".{target.name}.", suffix=".tmp", delete=False,
            ) as stream:
                pending = Path(stream.name)
                stream.write(rendered)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(pending, target)
        finally:
            if pending is not None:
                pending.unlink(missing_ok=True)
    sys.stdout.write(rendered)


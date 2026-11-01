"""Use RustHouse's validated snapshot metadata to enforce a physical row floor.

Metadata describes rows on disk, but a COUNT(*) may use metadata-only paths.
"""
import argparse
import json
import math
import os
import subprocess
import tempfile
from pathlib import Path

MAX_SNAPSHOT_REPORT_BYTES = 2 * 1024 * 1024


def reject_nonstandard_constant(value):
    raise ValueError(f"nonstandard snapshot JSON constant: {value}")


def inspect_snapshot(binary, path, *, min_rows=1_000_000_000, run=None,
                     max_parts=None, timeout=600):
    if (type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0):
        raise ValueError("snapshot timeout must be positive and finite")
    if type(min_rows) is not int or not 1 <= min_rows <= (1 << 64) - 1:
        raise ValueError("min_rows must be a positive UInt64 integer")
    if max_parts is not None and (
            type(max_parts) is not int or not 1 <= max_parts <= 16_384):
        raise ValueError("max_parts must be in 1..16384")
    command = [str(binary), "inspect-snapshot", str(path), "--json"]
    if run is None:
        extra = {}
        if os.name == "posix":
            import resource

            def limit_child_files():
                _, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
                cap = MAX_SNAPSHOT_REPORT_BYTES + 1
                if hard != resource.RLIM_INFINITY:
                    cap = min(cap, hard)
                resource.setrlimit(resource.RLIMIT_FSIZE, (cap, hard))

            extra["preexec_fn"] = limit_child_files

        with tempfile.TemporaryFile(mode="w+b") as output, tempfile.TemporaryFile() as errors:
            extra = {}
            if os.name == "posix":
                import resource
                def cap_report_file():
                    soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
                    limit = MAX_SNAPSHOT_REPORT_BYTES + 1
                    if soft != resource.RLIM_INFINITY:
                        limit = min(limit, soft)
                    if hard != resource.RLIM_INFINITY:
                        limit = min(limit, hard)
                    resource.setrlimit(resource.RLIMIT_FSIZE, (limit, hard))
                extra["preexec_fn"] = cap_report_file
            result = subprocess.run(
                command, stdout=output, stderr=errors,
                check=False, timeout=timeout, **extra,
            )
            output.seek(0, os.SEEK_END)
            if output.tell() > MAX_SNAPSHOT_REPORT_BYTES:
                raise ValueError("snapshot report exceeds 2 MiB")
            output.seek(0)
            payload = output.read(MAX_SNAPSHOT_REPORT_BYTES + 1)
            errors.seek(max(0, errors.tell() - 600))
            diagnostic = errors.read(600).decode("utf-8", "replace")
    else:
        result = run(command, capture_output=True, check=False, timeout=timeout)
        payload = result.stdout
        diagnostic = result.stderr[-600:].decode("utf-8", "replace")
    if len(payload) > MAX_SNAPSHOT_REPORT_BYTES:
        raise ValueError("snapshot report exceeds 2 MiB")
    if result.returncode:
        raise RuntimeError(f"snapshot inspection failed: {diagnostic}")
    def unique_fields(pairs):
        fields = {}
        for name, value in pairs:
            if name in fields:
                raise ValueError(f"duplicate snapshot JSON field: {name}")
            fields[name] = value
        return fields

    data = json.loads(payload, object_pairs_hook=unique_fields,
                      parse_constant=reject_nonstandard_constant)
    if not isinstance(data, dict):
        raise ValueError("snapshot report must be a JSON object")
    rows, parts = data.get("rows"), data.get("parts")
    if (type(rows) is not int or type(parts) is not int or rows < 0
            or rows > (1 << 64) - 1 or not 1 <= parts <= 16_384):
        raise ValueError("snapshot report has invalid rows or parts")
    if max_parts is not None and parts > max_parts:
        raise ValueError(f"snapshot parts {parts} above max_parts {max_parts}")
    columns = data.get("columns")
    if (not isinstance(columns, list) or not 1 <= len(columns) <= 128 or any(
        not isinstance(column, dict)
        or not isinstance(column.get("name"), str) or not column["name"]
        or not isinstance(column.get("type"), str) or not column["type"]
        for column in columns
    )):
        raise ValueError("snapshot report has invalid columns")
    if len({column["name"] for column in columns}) != len(columns):
        raise ValueError("snapshot report has duplicate columns")
    if rows < min_rows:
        raise ValueError(f"physical rows {rows} below required {min_rows}")
    return {"workload": "physical-snapshot-gate", "physical_rows": rows,
            "parts": parts, "columns": data["columns"], "minimum_rows": min_rows,
            "maximum_parts": max_parts,
            "verified_by": "rusthouse inspect-snapshot metadata", "physical_scan_timed": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--min-rows", type=int, default=1_000_000_000)
    parser.add_argument("--max-parts", type=int)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = inspect_snapshot(args.binary, args.part, min_rows=args.min_rows,
                                  max_parts=args.max_parts, timeout=args.timeout)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired, json.JSONDecodeError) as error:
        parser.exit(1, f"physical snapshot gate failed: {error}\n")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Plan billion-row bulk ingestion under RustHouse's current part-count limit."""
import argparse
import json
import math


def plan(rows, *, rows_per_part=65536, max_parts=16384, bytes_per_row=16,
         disk_budget_bytes=None, disk_copies=2):
    if type(rows) is not int or not 1 <= rows <= (1 << 64) - 1:
        raise ValueError("rows must be positive UInt64")
    if type(rows_per_part) is not int or not 1 <= rows_per_part <= 1_000_000:
        raise ValueError("rows_per_part must be in 1..1000000")
    if type(max_parts) is not int or not 1 <= max_parts <= 16384:
        raise ValueError("max_parts must be in 1..16384")
    if type(bytes_per_row) is not int or bytes_per_row < 1:
        raise ValueError("bytes_per_row must be a positive integer")
    if disk_budget_bytes is not None and (type(disk_budget_bytes) is not int or disk_budget_bytes < 1):
        raise ValueError("disk_budget_bytes must be positive")
    if type(disk_copies) is not int or not 1 <= disk_copies <= 8:
        raise ValueError("disk_copies must be in 1..8")
    required_parts = (rows - 1) // rows_per_part + 1
    minimum_rows = (rows - 1) // max_parts + 1
    estimated_bytes = rows * bytes_per_row
    part_fit = required_parts <= max_parts
    estimated_peak_bytes = estimated_bytes * disk_copies
    disk_fit = disk_budget_bytes is None or estimated_peak_bytes <= disk_budget_bytes
    return {
        "workload": "rusthouse-billion-row-capacity-plan",
        "rows": rows, "rows_per_part": rows_per_part,
        "parts_required": required_parts, "parts_available": max_parts,
        "minimum_rows_per_part_to_fit": minimum_rows,
        "bytes_per_row_assumption": bytes_per_row,
        "estimated_uncompressed_bytes": estimated_bytes,
        "disk_copies_assumption": disk_copies,
        "estimated_peak_disk_bytes": estimated_peak_bytes,
        "disk_budget_bytes": disk_budget_bytes,
        "part_limit_fit": part_fit, "disk_budget_fit": disk_fit,
        "feasible_under_assumptions": part_fit and disk_fit,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--rows-per-part", type=int, default=65536)
    parser.add_argument("--max-parts", type=int, default=16384)
    parser.add_argument("--bytes-per-row", type=int, default=16)
    parser.add_argument("--disk-copies", type=int, default=2)
    parser.add_argument("--disk-budget-bytes", type=int)
    parser.add_argument("--output")
    args = parser.parse_args(argv)
    try:
        report = plan(args.rows, rows_per_part=args.rows_per_part,
                      max_parts=args.max_parts, bytes_per_row=args.bytes_per_row,
                      disk_budget_bytes=args.disk_budget_bytes,
                      disk_copies=args.disk_copies)
    except ValueError as error:
        parser.exit(1, f"capacity plan failed: {error}\n")
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        from pathlib import Path
        Path(args.output).write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if report["feasible_under_assumptions"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

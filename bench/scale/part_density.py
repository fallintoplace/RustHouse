"""Summarize count.txt row density in a frozen wide-part directory.

Input must already be a vetted active-parts directory; this is not a substitute
for rusthouse inspect-snapshot validation.
"""
import argparse
import json
import statistics
from pathlib import Path

MAX_PARTS = 16_384


def part_density(root):
    root = Path(root)
    if not root.is_dir() or root.is_symlink():
        raise ValueError("expected a frozen snapshot directory")
    rows = []
    for child in root.iterdir():
        if child.is_symlink():
            raise ValueError(f"refusing symlink: {child.name}")
        count_file = child / "count.txt"
        if count_file.is_symlink():
            raise ValueError(f"refusing symlinked count.txt: {child.name}")
        if not child.is_dir() or not count_file.is_file():
            continue
        with count_file.open("rb") as source:
            payload = source.read(23)
        if len(payload) > 22:
            raise ValueError(f"invalid oversized count.txt in {child.name}")
        raw = payload.decode("ascii")
        digits = raw.removesuffix("\n").removesuffix("\r")
        if not digits or not digits.isascii() or not digits.isdecimal():
            raise ValueError(f"invalid count.txt in {child.name}")
        count = int(digits)
        if count <= 0 or count > (1 << 64) - 1:
            raise ValueError(f"out-of-range part rows in {child.name}")
        rows.append(count)
        if len(rows) > MAX_PARTS:
            raise ValueError("more than 16384 parts")
    if not rows:
        raise ValueError("no wide parts with count.txt")
    rows.sort()
    total = sum(rows)
    if total > (1 << 64) - 1:
        raise ValueError("part row totals exceed UInt64")
    return {"workload": "part-density-report", "part_count": len(rows),
            "rows": total, "min_rows_per_part": rows[0],
            "median_rows_per_part": statistics.median(rows),
            "max_rows_per_part": rows[-1],
            "mean_rows_per_part": total / len(rows),
            "parts_below_8192_rows": sum(value < 8192 for value in rows),
            "part_limit_remaining": MAX_PARTS - len(rows),
            "validated_active_set": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--part-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = part_density(args.part_dir)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, ValueError, UnicodeError) as error:
        parser.exit(1, f"part density failed: {error}\n")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

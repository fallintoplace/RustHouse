#!/usr/bin/env python3
"""Inventory SQL constructs in the pinned TPC-H corpus, not engine support."""
import argparse
import json
import re
from collections import Counter
from pathlib import Path

from official_coverage import clean_sql

PATTERNS = {
    "with_cte": r"\bWITH\s+[A-Z_][\w]*\s+AS\s*\(",
    "derived_table": r"\b(?:FROM|JOIN)\s*\(\s*SELECT\b",
    "nested_select": r"\(\s*SELECT\b",
    "exists": r"\bEXISTS\s*\(",
    "not_exists": r"\bNOT\s+EXISTS\s*\(",
    "in_subquery": r"\bIN\s*\(\s*SELECT\b",
    "count_distinct": r"\bCOUNT\s*\(\s*DISTINCT\b",
    "case": r"\bCASE\b",
    "extract_year": r"\bEXTRACT\s*\(\s*YEAR\s+FROM\b",
    "substring": r"\bSUBSTRING\s*\(",
    "outer_join": r"\b(?:LEFT|RIGHT|FULL)\s+(?:OUTER\s+)?JOIN\b",
    "having": r"\bHAVING\b",
    "like": r"\bLIKE\b",
    "group_by": r"\bGROUP\s+BY\b",
}
COMPILED = {label: re.compile(pattern, re.I) for label, pattern in PATTERNS.items()}


def mask_literals(sql):
    """Hide comments and quoted strings so SQL-looking text is not parsed as keywords."""
    out = []
    i = 0
    while i < len(sql):
        if sql.startswith("--", i):
            stop = sql.find("\n", i)
            if stop < 0:
                break
            out.extend(" " * (stop - i))
            i = stop
        elif sql.startswith("/*", i):
            stop = sql.find("*/", i + 2)
            if stop < 0:
                raise ValueError("unterminated SQL block comment")
            out.extend(" " * (stop + 2 - i))
            i = stop + 2
        elif sql[i] in ("'", '"'):
            quote = sql[i]
            start = i
            i += 1
            while i < len(sql):
                if sql[i] == quote:
                    if i + 1 < len(sql) and sql[i + 1] == quote:
                        i += 2
                        continue
                    i += 1
                    break
                i += 1
            else:
                raise ValueError("unterminated SQL quoted literal")
            out.extend(" " * (i - start))
        else:
            out.append(sql[i])
            i += 1
    return "".join(out)


def features(sql):
    source = mask_literals(sql)
    found = [label for label, pattern in COMPILED.items() if pattern.search(source)]
    if re.search(r"\bFROM\s+[A-Z_]\w*\s*,", source, re.I):
        found.append("comma_join")
    return sorted(found)


def inventory(queries_dir):
    queries_dir = Path(queries_dir)
    counts = Counter()
    rows = []
    for number in range(1, 23):
        path = queries_dir / f"q{number:02d}.sql"
        sql = clean_sql(path)
        tags = features(sql)
        counts.update(tags)
        rows.append({"query": number, "features": tags})
    return {"workload": "tpch-sql-feature-inventory", "official_tpch_score": False,
            "total_queries": 22, "feature_counts": dict(sorted(counts.items())),
            "results": rows}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--queries-dir", type=Path,
                        default=Path(__file__).resolve().parent / "queries")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = inventory(args.queries_dir)
    except (OSError, ValueError, UnicodeError) as error:
        parser.exit(1, f"TPC-H SQL feature inventory failed: {error}\n")
    payload = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(payload, encoding="utf-8")
    print(payload, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

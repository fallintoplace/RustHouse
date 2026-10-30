#!/usr/bin/env python3
"""Generate deterministic, integer-scaled analytical revenue fixtures.

This is TPC-H-inspired synthetic data, NOT official dbgen data.
"""
import argparse
import random
from datetime import date, timedelta
from pathlib import Path

SCHEMA = (
    "CREATE TABLE lineitem (l_shipdate Date, l_extendedprice UInt64, "
    "l_discount UInt64, l_quantity UInt64)"
)
QUERY = (
    "SELECT SUM(l_extendedprice * l_discount) AS revenue_bps, "
    "COUNT(*) AS matched_rows FROM lineitem "
    "WHERE l_shipdate >= '1994-01-01' AND l_shipdate < '1995-01-01' "
    "AND l_discount BETWEEN 500 AND 700 AND l_quantity < 24"
)
START = date(1993, 1, 1)
STOP = date(1996, 12, 31)
DAYS = (STOP - START).days

def row_source(rows, seed):
    rng = random.Random(seed)
    # Ensure a nonempty revenue result even for tiny smoke fixtures.
    yield ("1994-06-15", 100_000, 600, 10)
    for _ in range(rows - 1):
        shipdate = START + timedelta(days=rng.randint(0, DAYS))
        price = rng.randint(100, 10_000_000)
        discount = rng.randint(0, 10) * 100  # basis points
        quantity = rng.randint(1, 50)
        yield (shipdate.isoformat(), price, discount, quantity)

def generate(output_dir, rows, seed):
    if rows < 1:
        raise ValueError("rows must be positive")
    output_dir = Path(output_dir)
    (output_dir / "queries").mkdir(parents=True, exist_ok=True)
    (output_dir / "references").mkdir(parents=True, exist_ok=True)
    count, revenue = 0, 0
    with (output_dir / "lineitem.tsv").open("w", encoding="utf-8", newline="\n") as stream:
        stream.write("l_shipdate\tl_extendedprice\tl_discount\tl_quantity\n")
        pending = []
        for shipdate, price, discount, quantity in row_source(rows, seed):
            pending.append(f"{shipdate}\t{price}\t{discount}\t{quantity}\n")
            if len(pending) == 4096:
                stream.write("".join(pending))
                pending.clear()
            if ("1994-01-01" <= shipdate < "1995-01-01"
                    and 500 <= discount <= 700 and quantity < 24):
                revenue += price * discount
                count += 1
        if pending:
            stream.write("".join(pending))
    (output_dir / "queries" / "q06.sql").write_text(QUERY + ";\n", encoding="utf-8")
    (output_dir / "references" / "q06.tsv").write_text(
        f"{revenue}\t{count}\n", encoding="utf-8"
    )
    # Separate one-statement files avoid cross-statement transaction assumptions.
    (output_dir / "create.sql").write_text(SCHEMA + ";\n", encoding="utf-8")
    source = (output_dir / "lineitem.tsv").resolve().as_posix()
    source = source.replace("'", "''")
    (output_dir / "copy.sql").write_text(
        f"COPY lineitem FROM '{source}' WITH (FORMAT TSV, HEADER TRUE);\n",
        encoding="utf-8",
    )
    return {"rows": rows, "matched_rows": count, "revenue_bps": revenue}

def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--rows", type=int, default=60_000)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args(argv)
    try:
        report = generate(args.output_dir, args.rows, args.seed)
    except (OSError, ValueError) as exc:
        parser.error(str(exc))
    print(f"generated {report['rows']} rows; {report['matched_rows']} match Q6")
    print(f"expected scaled revenue: {report['revenue_bps']}")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())

# Real billion-row RustHouse benchmarks

Use an existing frozen snapshot with **physical** immutable wide parts. By
default, this benchmark requires at least one billion stored rows.

## Filtered COUNT (backwards compatible)

```bash
cargo bench -p rusthouse-query --bench billion_parts -- \
  --part /path/to/frozen/snapshot --filter 'id >= 500000000' \
  --expected-count 500000000 --samples 7 --warmups 2
```

## Stored SUM, GROUP BY, and Top-K

Write the expected tab-separated output for your SQL query to a file. The
benchmark checks **every** warmup and sample against that file.

```bash
printf 'sum(id)\n499999999500000000\n' > /tmp/rusthouse-sum.expected
cargo bench -p rusthouse-query --bench billion_parts -- \
  --part /path/to/frozen/snapshot \
  --sql 'SELECT sum(id) FROM part' \
  --expected-file /tmp/rusthouse-sum.expected \
  --samples 7 --warmups 2 --max-median-seconds 3
```

The example output is correct **only** when the snapshot contains every integer
from 0 through 999999999 exactly once. For other data, calculate the expected
output independently from that snapshot.

Use the same `--sql` / `--expected-file` flags to measure filtered
aggregates, GROUP BY, and ORDER BY with LIMIT. All executions must finish
within RustHouse's current query limits.

Use `--max-median-seconds` to fail a run that misses a time target. When you
share results, include p50, p90, CPU count, memory, storage type, data layout,
selected columns, compression, cache state, and part count.

"Logical rows/s" counts input rows, not bytes read from disk. A count answered
from metadata is not a physical scan of one billion rows.

# Real billion-row RustHouse benchmarks

Use an existing **physical** immutable wide-part snapshot. This benchmark
refuses snapshots smaller than one billion rows by default.

## Filtered COUNT (backwards compatible)

```bash
cargo bench -p rusthouse-query --bench billion_parts -- \
  --part /path/to/frozen/snapshot --filter 'id >= 500000000' \
  --expected-count 500000000 --samples 7 --warmups 2
```

## Stored SUM, GROUP BY, and Top-K

Write the exact expected tab-separated output for the chosen SQL to a file.
The benchmark checks **every** warmup and sample against those bytes.

```bash
printf 'sum(id)\n499999999500000000\n' > /tmp/rusthouse-sum.expected
cargo bench -p rusthouse-query --bench billion_parts -- \
  --part /path/to/frozen/snapshot \
  --sql 'SELECT sum(id) FROM part' \
  --expected-file /tmp/rusthouse-sum.expected \
  --samples 7 --warmups 2 --max-median-seconds 3
```

The sample SUM oracle is valid **only** if the snapshot contains each integer
from 0 through 999999999 exactly once. Use an oracle computed independently
from the actual snapshot for any other dataset.

Use the same `--sql` / `--expected-file` flags to measure filtered
aggregates, GROUP BY, and ORDER BY with LIMIT. All executions must finish
within RustHouse's current query limits.

The optional `--max-median-seconds` fails the benchmark if it misses the
target. Report both p50 and p90 plus CPU count, memory, storage type, data
layout, selected columns, compression, warm-cache state, and number of parts.
"Logical rows/s" describes input rows, **not** bytes read from disk. Do not
interpret metadata-only counts as physical billion-row scans.

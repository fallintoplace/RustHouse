# Benchmark real billion-row RustHouse snapshots

Use an **existing, immutable** MergeTree wide-part snapshot containing at
least one billion real rows. Benchmark creation and validation are separate
from the timed SQL scans.

```sh
cargo bench -p rusthouse-query --bench billion_parts --locked -- \
  --part /data/frozen_parts \
  --filter 'id >= 0' \
  --expected-count 1000000000 \
  --samples 7 --warmups 2
```

Provide the correct filtered count independently through
`--expected-count`. The harness checks that **every** timed query returns the
expected value and refuses a result mismatch. It reports median and p90 query
latency, row and part counts, and logical input rows per second.

For a quick development fixture use `--min-rows 1` to bypass the billion-row
floor. This does not claim billion-row performance.

The snapshot must be frozen, not a live merge directory. Only query execution
is timed; building data and parsing SQL are not timed. Warm-cache results
are affected by hardware and the OS page cache. Logical row throughput is
not actual bytes read and should not be confused with storage scan bandwidth.

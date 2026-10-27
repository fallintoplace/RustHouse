# RustHouse frozen-part scan benchmark

Run both physical layouts with a pinned Rust toolchain on the same machine:

```sh
cargo bench -p rusthouse-query --bench parts --locked -- --rows 65536 --samples 11 --layout sorted
cargo bench -p rusthouse-query --bench parts --locked -- --rows 65536 --samples 11 --layout shuffled
cargo bench -p rusthouse-query --bench parts --locked -- --rows 65536 --samples 11 --layout shuffled --csv > shuffled.csv
```

The **sorted** layout writes ascending `id` values across and within parts. The
**shuffled** layout uses a fixed-seed, deterministic Fisher-Yates permutation
of the same unique values, while preserving identical table schema, row count
and part sizes. Benchmark results can therefore be compared without changing
filter selectivity. Large OFFSET expectations use physical scan order.

The benchmark creates immutable UInt64 wide parts before timing, warms each
query three times, parses SQL before timing, and verifies every result. The
report includes median and p90 elapsed milliseconds plus millions of logical
input rows per second. `--csv` emits a header and comma-separated rows for
later comparisons. Default options retain the original sorted layout.

Workloads cover COUNT at approximately 100%, 50%, 10% and 1% selectivity,
filtered SUM, an exact point predicate, large OFFSET, and filtered top-K.
Neither layout changes ClickHouse-style primary-index pruning: RustHouse does
not yet use the primary index in normal SQL scans. Selectivity is **not** the
fraction of bytes read.

The timing includes filesystem reads under normal OS caching, but does not
force a cold cache or include fixture creation, SQL parsing, terminal rendering,
HTTP overhead, or concurrency. These numbers are not ClickHouse or DuckDB
comparisons. Cross-engine claims require identical data, query semantics,
hardware, cache controls, and measured bytes read.

# RustHouse frozen-part scan benchmark

Run on a dedicated machine with a pinned Rust toolchain and the same dataset size:

```sh
cargo bench -p rusthouse-query --bench parts --locked -- --rows 65536 --samples 11
cargo bench -p rusthouse-query --bench parts --locked -- --rows 1048576 --samples 11
```

The benchmark creates immutable UInt64 wide parts before timing, warms each
query three times, parses SQL before timing, and checks every result. Results
include the median and p90 elapsed milliseconds from an odd number of samples.

Count scans use predicates matching approximately 100%, 50%, 10%, and 1% of
rows, so future primary-index or predicate-pushdown work has a stable baseline.
Today, RustHouse does not prune primary-index ranges in these SQL scans.
Their selectivity is *not* equivalent to the fraction of bytes read.

The output's millions of **logical input rows per second** is not a measure of
actual bytes decoded. Runs include filesystem reads under normal OS caching,
but neither force a cold cache nor include fixture creation, SQL parsing,
terminal rendering, the HTTP server, or concurrency. The benchmark does not
establish a performance advantage over ClickHouse or DuckDB. Any cross-engine
comparison must use identical data, matching result semantics, controlled
hardware/cache settings, and reported dataset sizes.

# Bounded star-schema benchmark

This workload exercises a two-table analytical join using synthetic data.
It is inspired by the Star Schema Benchmark (SSB) but is **not** a full SSB
implementation or a comparable official benchmark score.

The fixture generates up to 1,000,000 fact rows using bounded Python generator
memory. The date dimension contains 30 rows. Ordered top-K uses the streaming
INNER JOIN engine for large fact inputs, while exact COUNT scans all joined
rows. Grouped revenue + transaction counts require the fused joined metrics
executor.

Generate data:

    python3 bench/ssb/fixture.py --output-dir /tmp/rusthouse-star --rows 2000

The fixture contains a SQLite reference, TSV tables, and RustHouse CREATE SQL.
The data generator is deterministic for the same row count and seed.

Large physical-data queries against an independent SQLite oracle:

    python3 bench/ssb/run.py --binary target/release/rusthouse --rows 1000000 --query count-join --runs 3 --timeout 600
    python3 bench/ssb/run.py --binary target/release/rusthouse --rows 1000000 --query grouped-revenue --runs 3 --timeout 600

The grouped-revenue option requires the fused grouped JOIN SUM/COUNT feature.
Each query checks its complete returned result against SQLite before recording
latency. Timings exclude fixture generation and ingestion. These synthetic
queries are not official SSB scores.

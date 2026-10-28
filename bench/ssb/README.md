# Bounded star-schema benchmark

This workload exercises a two-table analytical join using synthetic data.
It is inspired by the Star Schema Benchmark (SSB) but is **not** a full SSB
implementation or a comparable official benchmark score.

The fixture can generate up to 1,000,000 fact rows without holding a Python
list of the table in memory. The date dimension has 30 rows. Small
queries keep their existing bounded join plan. Large fact tables rely
on the bounded top-K streaming ordered INNER join executor when
requesting a small sorted result.

Generate data:

    python3 bench/ssb/fixture.py --output-dir /tmp/rusthouse-star --rows 2000

The fixture contains a SQLite reference, TSV tables, and RustHouse CREATE SQL.
The data generator is deterministic for the same row count and seed.

To generate a million-row physical star-schema snapshot for load and sorted
JOIN latency measurements:

    python3 bench/ssb/fixture.py --output-dir /large/fast/disk/ssb-million --rows 1000000
    python3 -m bench workload ssb -- --binary target/release/rusthouse --rows 1000000 --runs 5

The separate RustHouse database and SQLite oracle are constructed by the
benchmark runner. Ingestion and reference generation are not included in timed
JOIN latency. This is not a certified SSB or TPC-H performance result.

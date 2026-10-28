# Bounded star-schema benchmark

This workload exercises a two-table analytical join using synthetic data.
It is inspired by the Star Schema Benchmark (SSB) but is **not** a full SSB
implementation or a comparable official benchmark score.

The fact input is limited to 12,000 rows. The date dimension has 30 rows.
This keeps ordered joins below RustHouse's 16,384-row input limit.

Generate data:

    python3 bench/ssb/fixture.py --output-dir /tmp/rusthouse-star --rows 2000

The fixture contains a SQLite reference, TSV tables, and RustHouse CREATE SQL.
The data generator is deterministic for the same row count and seed.

# NYC taxi SQL coverage and DuckDB parity

Reproduce the ten real-world SQL queries against both engines. The SQL is pinned
in `queries.json`, including the four queries that failed at RustHouse
`46938032` on November 3, 2026.

The user-supplied fixture contained 2,789,022 NYC taxi trips. The initial
warm-cache release build passed 6/10 queries, each checked against DuckDB.
The successful CLI medians over three repeats, including process startup and
output formatting, were 130, 161, 157, 137, 225, and 105 milliseconds
for queries 1, 2, 3, 4, 5, and 10 respectively.

This benchmark requires that **both databases already contain identical**
`trips` and `zones` tables with the SQL column names in the manifest.
This script does not download data or alter either database.

## Usage

Install the Python DuckDB package, build RustHouse in release mode, and run:

```sh
python -m pip install duckdb
cargo build --release -p rusthouse-cli
python -m bench.nyc_taxi.compare \
  --rusthouse-bin ./target/release/rusthouse \
  --rusthouse-db /path/to/rusthouse-db \
  --duckdb-db /path/to/taxi.duckdb \
  --repeats 3
```

Run `python -m unittest bench.nyc_taxi.test_compare` for dependency-free tests.

The runner warms every successful SQL once, measures three complete RustHouse
CLI runs by default, and checks the last output against the DuckDB result.
It compares rows in ORDER BY order with a relative numeric tolerance of 1e-6,
and shows the first mismatch. It exits nonzero for SQL failures or mismatches.
Reported times **are not official benchmarks** and are not comparisons to
ClickHouse. Different datasets, cache state, or hardware cannot be compared
directly.

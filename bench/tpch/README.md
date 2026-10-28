# SQL analytical workload runner

Run a directory of qNN.sql query files against the local RustHouse CLI.
Results include query errors, optional byte-for-byte reference comparisons,
and median/p90 end-to-end CLI latency. This is not a certified TPC-H result.

Example:

    cargo build --release --locked -p rusthouse-cli
    python3 bench/tpch/run.py --binary target/release/rusthouse \
      --database /path/to/db --queries-dir /path/to/queries \
      --reference-dir /path/to/expected --runs 5 --warmups 2

Expected reference files are q01.tsv, q02.tsv, etc. The optional
--expected-lineitem-rows flag checks input size before timing.
Queries without references are labeled unverified rather than passed.
The JSON coverage summary reports successful execution separately from
byte-verified correctness. Unverified results never count as verified.
Time includes per-query CLI startup and operating system cached reads.
Measure load time, cold-cache behavior, and memory separately.

Unit tests:

    python3 -m unittest discover -s bench/tpch -p 'test_*.py'


## Pair cold CLI timings with DuckDB

Prepare the same logical tables in a RustHouse database and a DuckDB database.
Provide identical SQL files and independent headerless TSV references for both.
The runner refuses any query where either engine returns different bytes.

    python3 -m bench.tpch.paired_duckdb \
      --binary target/release/rusthouse --database /path/to/rusthouse-db \
      --duckdb duckdb --duckdb-database /path/to/analytics.duckdb \
      --queries-dir /path/to/qNN-sql --reference-dir /path/to/expected-tsv \
      --runs 5 --warmups 2 --output bench/results/paired-duckdb.json

Both CLIs start a new process for each sample. ABBA interleaves the engines
to reduce order bias. The output records query medians, raw measurements and
a geometric-mean DuckDB/RustHouse ratio **only if every result matches its
reference**. Ratios above 1 mean RustHouse has lower median latency.

These are local cold-process observations, not warm engine times or official
TPC-H numbers. Filesystem caches may be warm. Use identical SQL, data rows,
types, CPU and power settings. The tool does not prove that the supplied
databases contain identical data beyond the verified query outputs.

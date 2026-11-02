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

Parser-only inventory, without a populated database:

    python3 bench/tpch/parser_inventory.py --binary target/release/rusthouse \\
      --queries-dir bench/tpch/queries --output /tmp/tpch-parser-coverage.json

The parser inventory attempts all 22 queries via `--show-ast`. It does not
read TPC-H data and cannot establish whether the query executes correctly.
Use `official_coverage.py` and independent references for execution coverage.


## Unscaled Float64 compatibility fixture

The default dbgen converter stores money in integer cents and discounts in
integer basis points. Canonical TPC-H SQL expects fractional values, so that
fixture cannot be compared directly with native-decimal reference results.

For exploratory SQL compatibility, convert and load into a **new** RustHouse
database with `--numeric-mode float` on **both** commands:

    python3 bench/tpch/convert_dbgen.py --input-dir /path/to/dbgen --output-dir /tmp/tpch-float --numeric-mode float
    python3 bench/tpch/load_dbgen.py --binary target/release/rusthouse --database /tmp/tpch-float-db --input-dir /tmp/tpch-float --numeric-mode float

This uses unscaled Float64 values, not native exact SQL DECIMAL. It can reveal
parser/executor coverage on canonical SQL, but it is **not** a TPC-H-compliant
numeric schema. A scaled TSV must never be loaded as a float-mode fixture.

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

For exploratory Float64 fixtures only, `--comparison numeric` compares ordered
results using 1e-9 absolute and relative numeric tolerance. Text, headers,
row counts, and row ordering stay exact. This mode is not suitable for official
TPC-H accuracy claims or exact DECIMAL conformance. Exact remains the default.

When the SQL does not specify row order, use `--comparison unordered` to
compare output rows as a multiset (including duplicate counts). The default
is exact ordered TSV comparison. Both engines still must match the reference.

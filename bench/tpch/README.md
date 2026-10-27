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
Time includes per-query CLI startup and operating system cached reads.
Measure load time, cold-cache behavior, and memory separately.

Unit tests:

    python3 -m unittest discover -s bench/tpch -p 'test_*.py'

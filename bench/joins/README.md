# Two-table join workload

This workload measures a verified hash-join-shaped query with thousands
of build-side rows and a larger streaming probe-side table.

Example:

    cargo build --release -p rusthouse-cli --locked
    python3 bench/joins/run.py --binary target/release/rusthouse \
      --probe-rows 65536 --build-rows 8192 --runs 5 --warmups 2

The runner creates temporary tables, imports TSV, and verifies every
query result by checking matching IDs and their dimension labels.
Only SQL invocation latency is timed; fixture creation and loading are not.
The measured time includes process startup and output formatting.
This is not a standard TPC-H query and it does not establish cross-engine speed.

Unit tests:

    python3 -m unittest discover -s bench/joins -p 'test_*.py'

## Skewed fact keys

For a reference-verified hot-key JOIN workload with deterministic skew:

    python3 -m bench.joins.hotspot --binary target/release/rusthouse \
      --probe-rows 65536 --build-rows 1024 --hot-percent 90

The build side has unique dimension keys, while 90% of fact rows reuse a
single key. The output is checked as a multiset, including duplicate counts.
This measures fresh CLI SQL execution and does not compare with ClickHouse.

# Two-table join workload

This workload measures a verified hash-join-shaped query with thousands
of build-side rows and a larger streaming probe-side table.

Example:

    cargo build --release -p rusthouse-cli --locked
    python3 bench/joins/run.py --binary target/release/rusthouse \
      --probe-rows 65536 --build-rows 8192 --runs 5 --warmups 2

The runner creates temporary tables, imports TSV, and verifies every
query result by checking the complete set of matching IDs.
Only SQL invocation latency is timed; fixture creation and loading are not.
The measured time includes process startup and output formatting.
This is not a standard TPC-H query and it does not establish cross-engine speed.

Unit tests:

    python3 -m unittest discover -s bench/joins -p 'test_*.py'

# Physical billion-row SQL matrix

Run repeatable, exact-output probes against an **already frozen** RustHouse
wide-part snapshot. The snapshot must contain each UInt64 id in [0, rows)
**exactly once**. This prerequisite is not established by row-count metadata.

Build RustHouse, verify the source separately, then run:

    cargo build --release --locked -p rusthouse-cli
    python3 -m bench.scale.billion_matrix --binary target/release/rusthouse \
      --part /data/frozen-parts --rows 1000000000 --require-billion-rows \
      --runs 3 --warmups 1 --timeout 7200 \
      --output bench/results/billion-matrix.json

Pass --case CASE_ID repeatedly to select fewer queries. Without it the runner
executes all checked-in cases. Every warmup and timed sample is checked against
an independently calculated reference. A mismatch or resource limit fails the
run. Output is capped to 64 KiB per query, and all cases return small results.

The runner first checks SELECT count(*) against the supplied row count. This
may be metadata-only and **does not validate the actual id distribution**.
Filtered and ordered cases exercise persisted SQL paths, not numbers().

Reported wall times include launching a fresh CLI process and possible warm OS
cache; they are not cold-cache timings or proof of billion-row performance.
Compare only equivalent hardware, snapshots, versions, and query cases. Large
runs are explicitly opt-in and are not run in CI.

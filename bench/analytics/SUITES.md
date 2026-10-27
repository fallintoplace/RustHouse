# Synthetic, reference-verified analytics suites

The runner creates a fresh local RustHouse database with deterministic TSV
events. Each warmup and timed result must match SQLite on the same rows.

    cargo build --release --locked -p rusthouse-cli
    python3 -m bench workload analytics-suite -- \
      --binary target/release/rusthouse \
      --workload bench/analytics/workloads/baseline.json \
      --rows 10000 --runs 5 --warmups 2 \
      --output bench/results/analytics-baseline.json

Add a JSON workload under workloads/ to expand the matrix. The existing
Python test suite validates each committed query against SQLite.

Only per-query CLI startup, scans, and TSV serialization are timed. Fixture
generation, SQL reference evaluation, table creation, and COPY are excluded.
This is warm-cache synthetic data, not a certified TPC-H or cross-engine score.
Failures (errors, timeouts, or mismatches) produce a nonzero exit code.
Choose explicit ORDER BY keys for deterministic results. Nullable outputs and
floating-point results require a future type-aware comparison adapter.

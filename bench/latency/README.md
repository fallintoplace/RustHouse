# Verified SQL query latency

Run 20 deterministic, read-only SQL queries through the **release CLI**.
They cover counts, filters, numeric aggregates, DISTINCT, ordering, LIMIT,
OFFSET, grouped counts, and a stepped generated series. The input uses
numbers() and generate_series(), so no database, external data or fixture
import is necessary.

From the repository root:

```sh
cargo build --release --locked -p rusthouse-cli
python3 -m bench workload latency -- \
  --binary target/release/rusthouse \
  --rows 65536 --runs 5 --warmups 2 \
  --output bench/results/sql-latency.json
```

To check selected queries at a larger scale:

```sh
python3 -m bench workload latency -- \
  --binary target/release/rusthouse --rows 1000000 \
  --only 3,11-13,19-20 --runs 7 --warmups 2
```

Each query has a **separate Python-computed expected output**. Every
warmup and timed invocation must match the expected headerless TSV bytes.
Errors, mismatches and timeouts fail the command instead of presenting
misleading timings. Other queries continue after a failure.

The terminal displays median and nearest-rank p90 milliseconds per query.
The JSON report includes the SQL, all raw durations, status, machine
information, Rust compiler and Git revision. The benchmark workflow
automatically runs this suite on changes to its files and uploads the JSON
report as a GitHub Actions artifact.

**Timing scope:** wall-clock process startup, SQL parsing/execution and
TSV serialization. Fixture generation, Python oracle calculations and
warmups are excluded. This is warm-cache, single-query, single-process
CLI latency on generated data. It does **not** measure persistent table
scans, joins, cold-cache I/O or steady-state server execution. Some count
queries can use optimized execution paths. CI runner times can vary
considerably; compare meaningful performance changes on the same machine.

The existing `cargo bench -p rusthouse-query --bench queries` benchmark
complements this suite by timing execution **without** per-query CLI startup.

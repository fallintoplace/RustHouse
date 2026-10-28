# Verified physical wide-part scans

This workload first writes and loads real TSV data into a disposable RustHouse
database using `COPY FROM`. It verifies the physical snapshot with
`rusthouse doctor` before running COUNT and SUM scans.

Both analytical queries contain `WHERE id % 2 = 0`. Their independently
computed results depend on row values, not only the stored row-count metadata.
Every warmup and timed execution is checked. Failed ingestion, corrupted parts,
incorrect SQL results, or incomplete queries fail the run.

Build RustHouse before starting:

```sh
cargo build --release --locked -p rusthouse-cli
python3 -m bench.scale.physical_scan \
  --binary target/release/rusthouse --rows 1000000 \
  --runs 5 --warmups 2 --output bench/results/physical-million.json
```

For a true billion-row attempt, explicitly request the minimum row count:

```sh
python3 -m bench.scale.physical_scan \
  --binary target/release/rusthouse --rows 1000000000 \
  --require-billion-rows --timeout 7200 --scratch-dir /large/fast/disk \
  --output bench/results/physical-billion.json
```

This creates a very large temporary TSV file and database. Choose a scratch
filesystem with ample free space, and be aware that RustHouse may hit storage
part limits or other resource caps. Such a failure is a **failed experiment**,
not a benchmark success.

Measurements include a fresh CLI process for each query. Operating-system
caches may be warm; this is neither a cold-cache benchmark nor an official
ClickBench/TPC-H comparison. Ingestion time is recorded separately from
query time. This workload does not assert general billion-row SQL readiness.

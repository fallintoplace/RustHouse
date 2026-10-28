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


## Physical skewed GROUP BY on up to a billion rows

This workload persists a real two-column `UInt16`/`UInt64` fact table with
90% of events in one hot group and up to 4,096 groups overall. It validates
the published wide-part snapshot, then checks full
`GROUP BY count(*)` and `GROUP BY sum(amount)` results for **every group**
on **every timed run** against a separate modular arithmetic oracle. The
reference calculation does not scan the full input and handles a billion rows.

```sh
python3 -m unittest bench.scale.test_grouped_physical
python3 -m bench.scale.grouped_physical \
  --binary target/release/rusthouse --rows 1000000 --groups 4096 \
  --runs 5 --warmups 2 --output bench/results/grouped-million.json
python3 -m bench.scale.grouped_physical \
  --binary target/release/rusthouse --rows 1000000000 \
  --require-billion-rows --timeout 7200 --scratch-dir /large/fast/disk \
  --output bench/results/grouped-billion.json
```

At a billion rows, expect a large TSV input plus a large persisted database,
roughly 15,259 parts at a 65,536-row part size. Check disk space and file
descriptor limits first. A resource limit, incomplete scan, wrong result, or
failed import is a **failed gate**, not a successful performance measurement.

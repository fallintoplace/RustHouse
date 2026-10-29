# Persisted billion-row query matrix

A correctness-checked **physical table** workload for `events(id UInt64)`.
Each integer from `0` through `rows - 1` must occur **exactly once**. The
runner uses `rusthouse doctor` to verify the persisted database's structural
integrity and row count before launching any timed samples. This does **not**
independently prove that all IDs are present. Use the deterministic TSV/`COPY`
fixture procedure documented in `bench/scale/README.md` for that guarantee.

The matrix covers low and high selectivity modulo filters, head/middle/tail
ranges, membership and exclusions, range/predicate conjunctions, and bounded
`ORDER BY LIMIT/OFFSET`. The expected output for every case is computed in
constant additional space from exact integer arithmetic and checked for
**every warmup and every timed run**. No raw billion-row results are retained.

```sh
cargo build --release --locked -p rusthouse-cli
python3 -m unittest bench.scale.test_billion_matrix
python3 -m bench.scale.billion_matrix \
  --binary target/release/rusthouse \
  --database /mnt/rusthouse-billion-db --rows 1000000000 \
  --require-billion-rows --runs 1 --warmups 0 \
  --timeout 7200 --output bench/results/billion-matrix.json
```

The database is **read-only** to this runner. It must already exist and contain
`events(id UInt64)`. To reproduce the database, use the verified physical
fixture recipe in the scale README. For a local smoke test, use at least
`100000` rows with the same sequential-ID contract.

This is **not** a certified ClickBench or TPC-H result, a cold-cache test,
or evidence that all SQL and storage operations scale to billions of rows.
Actual hardware, part layout, file-system cache, disk space, and output
artifacts matter. On capacity-limited configurations even writing 2+ billion
rows may not succeed. Check `bench.scale.capacity_plan` before ingestion.

All 30 case files are separately reviewable and stack on the common runner;
existing engine optimizations can be assessed without changing case semantics.

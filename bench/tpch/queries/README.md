# Canonical TPC-H query corpus

The `q01.sql` through `q22.sql` files in this folder are the **full TPC-H query forms**, with fixed qualification values from [DuckDB](https://github.com/duckdb/duckdb/tree/6287a4e595d1c6c07d4241676385cbffbd232f07/extension/tpch/dbgen/queries), not RustHouse's simplified Q1/Q3/Q6/Q12 derived workloads. Source revision: `6287a4e595d1c6c07d4241676385cbffbd232f07`. The source attribution and MIT license are in `LICENSE`.

This directory contains SQL, not a claim of compatibility. The current RustHouse engine may reject some or most statements, and the existing `schema.py` loads modified fixed-point values, which **do not match the decimal values assumed by the original SQL**. A successful zero-row response on the converted schema is not a pass. Use a reference engine against equivalent logical data and verify every output before comparing latency.

Run the current RustHouse compatibility inventory after loading appropriate input data:

```sh
python3 bench/tpch/official_coverage.py \\
  --binary target/release/rusthouse \\
  --database /path/to/tpch-db \\
  --queries-dir bench/tpch/queries \\
  --reference-dir /path/to/reference-results \\
  --output /tmp/rusthouse-tpch-coverage.json
```

Only independent reference-verified results count as passed. The runner never produces a certified TPC-H score. See [the TPC-H 3.0.1 specification](https://www.tpc.org/tpc_documents_current_versions/current_specifications5.asp?mode=CURRENT-ONLY) and the [local runner](../README.md).

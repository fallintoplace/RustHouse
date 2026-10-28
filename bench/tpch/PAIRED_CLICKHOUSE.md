# Reference-checked RustHouse vs ClickHouse latency

Both databases must already contain **the same logical data**. Create reference
TSV files separately, one per SQL query, named `q01.tsv`, `q02.tsv`, etc.
The reference is the ground truth. A mismatch in either engine invalidates that
query's speed comparison. Do not fabricate reference values from the timed run.

Run against an existing ClickHouse server using the configured native client:

```sh
cargo build -p rusthouse-cli --release --locked
python3 -m bench.tpch.paired_clickhouse \
  --binary target/release/rusthouse \
  --database /path/to/rusthouse/database \
  --clickhouse clickhouse-client \
  --clickhouse-database default \
  --queries-dir /path/to/compatible/sql \
  --reference-dir /path/to/reference-tsv \
  --comparison exact \
  --warmups 2 --runs 7 \
  --output bench/results/paired-clickhouse.json
```

Configure ClickHouse connection and authentication using the normal client
configuration or environment. Keep the SQL files identical for both engines and
use deterministic ORDER BY clauses, or choose `--comparison unordered` for
queries whose ordering is not specified.

Each measured sample starts a **fresh RustHouse CLI** and a **fresh ClickHouse
client process** connected to a running server. The ABBA schedule reduces
order bias but **does not equalize process startup, TCP transport, server caches,
or memory residency**. All samples are correctness-checked. The geomean is
`null` unless every query passes. This is not an official TPC-H score or
a general engine-throughput comparison.

## Audit before publishing a speedup

Paired measurements must be reference verified. Check the raw samples,
per-query medians, p90 values, SQL fingerprint and geometric mean:

    python3 -m bench.tpch.competitive_gate \
      --input bench/results/paired-clickhouse.json \
      --min-queries 22 --min-geomean 1.0

Only use `--min-queries 22` when all 22 canonical query IDs were actually
run and independently verified. This is an optional, manually invoked audit
and does not turn the paired CLI comparison into an official TPC-H score.
A ratio above 1 measures the RustHouse speed advantage in this specific
fresh-process/native-client setup, not an in-process server comparison.

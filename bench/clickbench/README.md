# ClickBench projection-aware smoke runner

This is a **smoke test**, not a full or official ClickBench score. Supply a local
RustHouse database and the upstream ClickBench queries.sql file (43 SQL
statements, one per non-comment line). The runner never downloads the 16 GB+
source dataset.

From the repository root:

    cargo build --release --locked -p rusthouse-cli
    python3 -m bench workload clickbench -- \
      --binary target/release/rusthouse \
      --database /path/to/db \
      --queries-file /path/to/ClickBench/clickhouse/queries.sql \
      --projection-manifest bench/clickbench/three-column-projection.json \
      --reference-dir /path/to/sqlite-expected \
      --only 1,2,3,4,5,8,16,20,30 \
      --runs 7 --warmups 2 \
      --output bench/results/clickbench-projection.json

The projection manifest must be explicit. Example:

    {
      "available_columns": ["UserID", "AdvEngineID", "ResolutionWidth"],
      "query_columns": {
        "1": [],
        "2": ["AdvEngineID"],
        "3": ["AdvEngineID", "ResolutionWidth"],
        "4": ["UserID"]
      }
    }

The example is illustrative. The repository also ships a validated
three-column projection manifest for the official 43-query file: queries
1,2,3,4,5,8,16,20,30 are runnable, 33 queries need omitted columns, and
query 24 (SELECT *) is intentionally unknown because its full-schema
requirements cannot be described by the three-column projection.
Use the included manifest only when the table has exactly UserID, AdvEngineID
and ResolutionWidth; otherwise derive the manifest from your own schema.
An empty query list
means the query needs no physical input columns. A query is skipped as
skipped_missing_columns if its requirements mention absent columns.
Unmapped query numbers are unknown_requirements and are **not** counted as
database failures. Without a manifest, all queries are attempted, but
missing-column errors cannot be distinguished from unsupported SQL reliably.

Optional reference files are named q01.tsv, q02.tsv, etc. and contain exactly
the headerless TSV output of the expected query. Integer cells are checked
exactly (including integers over 2^53), and decimal/floating cells are checked
with relative tolerance 1e-12. Unverified runs are labeled unverified and
never counted as proven correct.

Statuses distinguish passed, unverified, skipped_missing_columns,
unknown_requirements, parse_error, schema_error, execution_error, mismatch,
timeout and missing_reference. The JSON summary reports eligible queries,
executed successes and verified results separately. The official query count
is 43; a shorter query file is explicitly marked incomplete. A subset selected
with --only retains its original query IDs.

All timings include CLI startup and cached filesystem reads. The runner warms
each query before measurement, records every measured sample, and computes
median and nearest-rank p90. The tool does not measure cold cache, ingest, peak
memory or official full-scale ClickBench performance.

Runner unit tests:

    python3 -m unittest discover -s bench/clickbench -p 'test_*.py'

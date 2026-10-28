# ClickHouse differential analytics verification

Run the same synthetic `events` and `accounts` dataset through RustHouse
and a running ClickHouse server. This is **result-parity verification**, not an
official performance score.

```bash
python3 -m bench.analytics.clickhouse_parity \
  --binary target/release/rusthouse \
  --client clickhouse-client \
  --workload bench/analytics/workloads/having-group-filter.json \
  --rows 10000 --host localhost --port 9000 \
  --output /tmp/rusthouse-ch-parity.json
```

The harness creates a fresh UUID-suffixed database and removes only that
database, including when an analytical query fails. No existing application
database or tables are touched. It loads identical rows and verifies record
counts before running queries. The default output comparison is ordered; a
workload can set `"unordered": true` when no ORDER BY is specified. Large
integers are compared exactly, floating results within 1e-12 tolerance, and
NULL/duplicate rows are checked.

The optional timing fields are measured over different CLIs and are **not**
comparable engine-only benchmark results. For meaningful performance work,
warm up both engines and measure scan time independently.

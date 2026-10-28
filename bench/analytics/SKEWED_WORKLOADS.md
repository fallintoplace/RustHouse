# Skewed analytical SQL workloads

The default fixture is unchanged. Add `--distribution skewed` to generate a
deterministic BI-style workload with multiple stress conditions:

- **Heavy hitters**: at least 80% of events occur across eight user IDs,
  with a separate long tail of hundreds of distinct customers
- **Correlated dimensions**: roughly 75% of records use the primary region
  and roughly 70% the dominant product category
- **High-value outliers**: approximately 1 out of 257 events has revenue
  1,000,000 instead of the normal 100-9,999 range
- **Sparse optional data**: about 91% of coupon values are NULL
- **Revenue skew**: approximately 70% of events are paid

```bash
python3 -m bench.analytics.suite_runner \
  --binary target/release/rusthouse \
  --workload bench/analytics/workloads/skewed-heavy-hitter-analytics.json \
  --distribution skewed --rows 200000 --runs 10 --warmups 3 \
  --output /tmp/rusthouse-skewed.json
```

The 12 queries compare exact distinct customer counts, top spender ordering,
regional/category sums, conversion-adjacent paid revenue and nullable promo
segments against SQLite with typed floating-value comparison. This detects
incorrect results, lost duplicate grouping rows, and unstable top-N behavior,
rather than reporting an attractive throughput number for a wrong answer.

Results include CLI startup and a warm filesystem cache. Compare with the
uniform fixture at the same row count and run settings, and treat both as
synthetic compatibility/performance diagnostics, not official ClickBench.

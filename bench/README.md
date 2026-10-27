# Centralized testing and benchmarks

Run these commands from the repository root with Python 3 and the Rust toolchain.

## Correctness checks

```sh
python3 -m bench check
python3 -m bench python-tests
```

`check` runs Rust formatting, Clippy, workspace tests, workspace build, and all
Python harness tests. `python-tests` covers the analytics, joins, measurements,
and TPC-H workload harnesses plus the shared runner itself. Each Python
suite is discovered separately because several have a `test_run.py` module.
Failing suites do not prevent the remaining suites from executing.

## Release benchmarks

```sh
python3 -m bench benchmark --rows 65536 --samples 7
python3 -m bench benchmark --suite parts-sorted --suite parts-shuffled --rows 65536 --samples 11
python3 -m bench benchmark --suite grouped --rows 65536 --stations 413
```

All four verified Rust workload variants run by default: generated queries,
sorted part scans, shuffled part scans, and grouped aggregates. The runner
requires a positive odd sample count, records each exact command and exit
code, and preserves stdout and stderr even when a suite fails. The default
output directory, `bench/results/`, contains `manifest.json` with Git
revision, compiler version, OS, architecture, logical CPU count, input
settings and elapsed runner times. Part-scan reports are CSV; other Rust
benchmark reports remain in their native text format. Use `--output-dir` to
store artifacts elsewhere. Results are not compared to CI performance
thresholds because shared-host timings are noisy.

Use the same compiler, CPU, power settings, sample count and data scale
when comparing results. Query execution benchmarks exclude SQL parsing
and fixture generation; part scans include normal OS cache effects.
These are not certified cross-engine benchmark results.

## End-to-end SQL workload runners

```sh
cargo build -p rusthouse-cli --release --locked
python3 -m bench workload latency -- --binary target/release/rusthouse --rows 65536 --runs 5 --warmups 2 --output bench/results/sql-latency.json
python3 -m bench workload joins -- --binary target/release/rusthouse --runs 5
python3 -m bench workload measurements -- --binary target/release/rusthouse --rows 100000
python3 -m bench workload analytics -- --binary target/release/rusthouse --database /path/to/db
python3 -m bench workload tpch -- --binary target/release/rusthouse --database /path/to/db --queries-dir /path/to/queries
```

These commands forward arguments to their Python runners. The new
[20-query generated SQL latency workload](latency/README.md) needs no
fixtures or database. It checks exact results on every run and reports
median/p90 wall-clock CLI latency. Its JSON report is also collected by
the SQL latency GitHub Actions workflow. Other workloads retain their
fixture and reference-checking options. Consult their README or `--help`.
Missing references or output mismatches are failures when reference
verification is requested. Without references, successful SQL execution
in the other workloads is explicitly marked `unverified`, not `passed`.

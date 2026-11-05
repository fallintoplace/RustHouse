# RustHouse

RustHouse is a SQL database written in Rust for local analytics. Create tables, import files, run analytical queries, and inspect frozen columnar data parts.

## What can I do with RustHouse?

- **Query data:** filter, sort, group, and join tables with SQL.
- **Store data locally:** create tables and import CSV, TSV, or JSONEachRow files.
- **Inspect data parts:** read and validate frozen parts.
- **Connect an app:** use the [read-only HTTP server](crates/rusthouse-server/README.md).
- **Choose an output format:** use TSV, CSV, JSON, Markdown, HTML, and more.

RustHouse supports local data workflows and offline analysis. The [SQL support guide](docs/reference.md#supported-features) describes the available syntax. The read-only HTTP server listens on loopback by default; see the [security guide](docs/security-threat-model.md) when configuring access from other machines.

## Quick start

Install **Rust 1.85 or newer**, then run this command from the repository root:

```sh
cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10) WHERE number >= 7 ORDER BY number DESC LIMIT 3"
```

You should see:

```text
number
9
8
7
```

## Create a local database

The `--database` option stores your tables in a local folder. Run these commands from the repository root:

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, name String)" --database ./mydb
cargo run -p rusthouse-cli -- sql "INSERT INTO events VALUES (1, 'Ada'), (2, 'Grace')" --database ./mydb
cargo run -p rusthouse-cli -- sql "SELECT id, name FROM events ORDER BY id" --database ./mydb
```

You should see:

```text
id	name
1	Ada
2	Grace
```

You can also import a CSV file:

```sh
cargo run -p rusthouse-cli -- sql "COPY events FROM '/path/to/events.csv' WITH (FORMAT CSV, HEADER TRUE)" --database ./mydb
```

Use table column names in the CSV header. TSV is supported for larger imports. See the [local database guide](docs/reference.md) for import formats, updates, deletes, and joins.

## Inspect a frozen data part

```sh
cargo run -p rusthouse-cli -- inspect-part /path/to/wide-part --json
cargo run -p rusthouse-cli -- sql "SELECT id, count(*) FROM part GROUP BY id ORDER BY id" --part /path/to/frozen-active-parts
```

Use a **stable, offline snapshot** for consistent inspection. See the [part guide](docs/reference.md#query-frozen-parts) for details.

## Check a database

Run the offline doctor when the database is idle and the HTTP server is stopped:

```sh
cargo run -p rusthouse-cli -- doctor --database ./mydb
```

Doctor validates your tables and reports its findings while preserving all data files. See the [doctor guide](docs/reference.md#diagnose-a-local-database-without-changing-it) for details and full hash checks.

## Benchmarks

Example timings from a release build at `7c450104` on an Apple M1 Pro with 32 GB RAM:

| Query | Data | Median time |
| --- | ---: | ---: |
| Group counts by region and category | 200,000 events | 21.8 ms |
| Group counts by user and rank them | 200,000 events, 997 users | 12.0 ms |
| Join four tables and return 20 rows | 16,000 events, 997 accounts | 41.6 ms |

Each result matched SQLite. Measurements use synthetic data and seven warm-cache runs after two warmups. Times include CLI startup and output; data generation and import are outside the timed run. Rust 1.96.0 was used. See the [benchmark guide](bench/README.md) and [reproduction commands](docs/reference.md#benchmarks).

All 22 canonical TPC-H queries matched reference results on a generated SF0.05 fixture with
299,814 lineitems (2026-11-05). The run used `Float64` values with `1e-9` absolute and relative
tolerance. See the [TPC-H runner guide](bench/tpch/README.md) for the fixture and reproduction commands.

## Learn more

- [SQL support](docs/reference.md#supported-features)
- [Local database and joins](docs/reference.md)
- [HTTP server](crates/rusthouse-server/README.md)
- [Examples](examples/demo/README.md)
- [Benchmarks](bench/README.md)
- [Recovery guide](docs/local-database-recovery-runbook.md)

## Development

Run all local checks:

```sh
python3 -m bench check
```

Run individual checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## License

RustHouse is licensed under the [Apache License, Version 2.0](LICENSE).

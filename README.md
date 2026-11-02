# RustHouse

RustHouse is an experimental SQL database written in Rust. Use it to create local tables, run analytical queries, and inspect frozen ClickHouse MergeTree parts.

## What can I do with RustHouse?

- **Query data:** filter, sort, group, and join tables with SQL.
- **Store data locally:** create tables and import CSV, TSV, or JSONEachRow files.
- **Inspect parts:** read and check frozen MergeTree parts.
- **Connect an app:** use the [read-only HTTP server](crates/rusthouse-server/README.md).
- **Choose an output format:** use TSV, CSV, JSON, Markdown, HTML, and more.

RustHouse is for local experiments and offline analysis. It supports a subset of SQL and is not ready for production use. The HTTP server has no built-in authentication and listens on loopback by default. See the [security guide](docs/security-threat-model.md) before sharing access.

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

Use table column names in the CSV header. For larger imports, TSV is also supported. See the [local database guide](docs/reference.md#local-sql-database-experimental) for import formats, updates, deletes, joins, and limits.

## Inspect a frozen MergeTree part

```sh
cargo run -p rusthouse-cli -- inspect-part /path/to/wide-part --json
cargo run -p rusthouse-cli -- sql "SELECT id, count(*) FROM part GROUP BY id ORDER BY id" --part /path/to/frozen-active-parts
```

Use a **frozen, offline snapshot**. Do not read a directory while another process is changing it. See the [part guide](docs/reference.md#query-frozen-parts) for details.

## Check a database

Stop writers and the HTTP server before running the offline doctor:

```sh
cargo run -p rusthouse-cli -- doctor --database ./mydb
```

Doctor checks your tables and reports problems. It does not repair or delete files. See the [doctor guide](docs/reference.md#diagnose-a-local-database-without-changing-it) for details and full hash checks.

## Benchmarks

On an Apple M1 Pro with 32 GB RAM, a release build at `7c450104` ran these queries:

| Query | Data | Median time |
| --- | ---: | ---: |
| Group counts by region and category | 200,000 events | 21.8 ms |
| Group counts by user and rank them | 200,000 events, 997 users | 12.0 ms |
| Join four tables and return 20 rows | 16,000 events, 997 accounts | 41.6 ms |

Each result matched SQLite. These are synthetic, warm-cache tests with seven runs after two warmups. Times include CLI startup and output, but not data generation or import. Rust 1.96.0 was used. See the [benchmark guide](bench/README.md) and [reproduction commands](docs/reference.md#benchmarks).

## Learn more

- [SQL features and limits](docs/reference.md#supported-features)
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

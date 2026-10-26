# RustHouse

**RustHouse is an experimental local Rust SQL database and offline MergeTree inspector.** It supports persistent catalog tables with bounded `INSERT ... VALUES` and `SELECT`, generated numbers, frozen wide-part queries, and a localhost HTTP server. **Not a production database.**

## Quick start

Requires **Rust 1.85+**. Run from the repository root:

```sh
cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10) WHERE number >= 7 ORDER BY number DESC LIMIT 3"
```

Output:

```text
number
9
8
7
```

## Query frozen parts

```sh
cargo run -p rusthouse-cli -- inspect-part /path/to/wide-part --json
cargo run -p rusthouse-cli -- sql "SELECT id, count(*) FROM part GROUP BY id ORDER BY id" --part /path/to/frozen-active-parts
```

`--part` accepts one wide part or a directory of compatible active wide parts. `part` is the SQL source for those files, not a catalog table.

## Supported features

- **SQL:** Local `CREATE TABLE`, bounded `INSERT INTO ... VALUES`, and `SELECT` from catalog tables. Queries also support `WHERE` (including `IN`, `BETWEEN`, `LIKE`, and `ILIKE`), sorting, `DISTINCT`, aggregates, grouped counts, and pagination. This is a limited SQL subset.
- **Data:** `numbers()` and inclusive `generate_series(start, stop[, step])` without files, plus supported numeric, `String`, `UUID`, and `Nullable` part columns.
- **Output:** TSV, CSV, JSON, JSON Lines, Markdown, and vertical formats via `--format`; also `--output`, `--max-rows`, `--time`, `--show-ast`, `--explain`, `--file`, and `--stdin`.
- **Inspection:** `verify-part`, `verify-file-sizes`, `inspect-marks`, `inspect-checksums`, `inspect-blocks`, and `inspect-primary-index`. Run `cargo run -p rusthouse-cli -- --help` for the full CLI.
- **HTTP:** an optional [read-only localhost server](crates/rusthouse-server/README.md), with no authentication or remote access.

## Local SQL tables

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, name String)" --catalog ./catalog
cargo run -p rusthouse-cli -- sql "INSERT INTO events VALUES (1, 'first'), (2, 'second')" --catalog ./catalog
cargo run -p rusthouse-cli -- sql "SELECT id, name FROM events WHERE id >= 2" --catalog ./catalog
cargo run -p rusthouse-cli -- sql "SHOW TABLES" --catalog ./catalog
```

Catalog schemas are stored as `.schema` files. Each INSERT creates an immutable wide MergeTree part in `<table>.parts/`, which later SELECT queries read with the existing executor. `INSERT` supports integer, floating-point, String, and Nullable columns, including explicit column lists. Missing columns must be Nullable. `DROP TABLE` removes local table parts, and `RENAME TABLE` moves them. Schema-altering operations on populated tables are rejected rather than silently corrupting existing parts.

## Safety and limits

**Only use frozen, offline snapshots with `--part`.** Never read a live upstream table directory while another process modifies it. Local catalog writes are experimental and intended for a single writer process; there is no full transaction isolation, crash recovery, or concurrent-DDL protection. RustHouse rejects overlapping part ranges, inconsistent schemas, and snapshots with more than 10,000 parts. Primary-index predicate pruning is not implemented.

## Benchmarks

Run the in-memory SQL execution benchmark in release mode:

```sh
cargo bench -p rusthouse-query --bench queries --locked -- --rows 100000 --samples 11
```

It measures filtered counts, filtered sums, and filtered top-K queries over generated `numbers()` rows. Each query is parsed before timing, warmed up three times, and checked for the expected result. The output shows median latency and millions of input rows per second.

This covers query execution and formatting into a memory buffer, **not** disk scans, terminal rendering, or server overhead. Compare runs on the same machine and Rust compiler version; shared CI runners are not suitable for stable performance numbers.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

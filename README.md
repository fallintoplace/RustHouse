# RustHouse

**RustHouse is an experimental local SQL database CLI and offline MergeTree inspector.** `--database` stores queryable tables on disk using immutable wide parts. It also queries generated numbers and frozen wide parts, and has separate metadata-only catalog and read-only localhost HTTP modes. **Not a production database.**

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

## Local SQL database

The `--database` mode supports a persistent **CREATE, INSERT, SELECT** workflow. Each command can run in a separate process:

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, message String, source Nullable(String))" --database ./mydb
cargo run -p rusthouse-cli -- sql "INSERT INTO events VALUES (1, 'started', NULL), (2, 'ready', 'cli')" --database ./mydb
cargo run -p rusthouse-cli -- sql "SELECT id, message FROM events WHERE id >= 1 ORDER BY id" --database ./mydb
```

Tables persist under `./mydb` as schema files and immutable checked wide parts. INSERT can use an explicit column list, and omitted `Nullable(...)` columns become `NULL`. SELECT supports the existing filtering, grouping, aggregation, sorting, and output formats, including `--format csv` and `--format json`.

**Current limits:** One writer at a time, one new part per INSERT statement, 4096-byte SQL statements, up to 4096 rows per insert, and 10000 parts per table (including the initial empty part). Writable columns support numeric, `String`, and their `Nullable` forms. There are no UPDATE, DELETE, ALTER, DROP, RENAME, JOIN, transactions, background merges, indexes, or multi-statement batches in database mode yet. The write lock is a local lock file; a crash can leave a stale lock file that requires manual recovery. This is a single-host SQL database prototype, not a production storage engine.

## Query frozen parts

```sh
cargo run -p rusthouse-cli -- inspect-part /path/to/wide-part --json
cargo run -p rusthouse-cli -- sql "SELECT id, count(*) FROM part GROUP BY id ORDER BY id" --part /path/to/frozen-active-parts
```

`--part` accepts one wide part or a directory of compatible active wide parts. `part` is the SQL source for those files, not a catalog table.

## Supported features

- **SQL:** `WHERE` (including `IN`, `BETWEEN`, `LIKE`, and `ILIKE`), sorting, `DISTINCT`, numeric aggregates (including `median`), `COUNT(column)` (skips NULLs), grouped counts with `HAVING`, and pagination. This is a limited SQL subset.
- **Data:** `numbers()` and inclusive `generate_series(start, stop[, step])` without files, plus supported numeric, `String`, `UUID`, and `Nullable` part columns. Writable database tables use supported numeric and `String` types.
- **Output:** TSV, CSV, JSON, JSON Lines, Markdown, and vertical formats via `--format`; also `--output`, `--max-rows`, `--time`, `--show-ast`, `--explain`, `--file`, and `--stdin`.
- **Inspection:** `verify-part`, `verify-file-sizes`, `inspect-marks`, `inspect-checksums`, `inspect-blocks`, and `inspect-primary-index`. Run `cargo run -p rusthouse-cli -- --help` for the full CLI.
- **HTTP:** an optional [read-only localhost server](crates/rusthouse-server/README.md), with no authentication or remote access.

## Local schema catalog

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, name String)" --catalog ./catalog
cargo run -p rusthouse-cli -- sql "SHOW TABLES" --catalog ./catalog
```

`CREATE`, `ALTER`, `RENAME`, `DROP`, `DESCRIBE`, and `SHOW CREATE TABLE` manage **local schema metadata only**. They do not modify MergeTree data, and catalog tables cannot be queried with `SELECT`. Use `--database` instead for persistent, queryable tables.

## Safety and limits

**Only use frozen, offline snapshots with `--part`.** Never read an upstream live table directory while another process modifies it. `--database` manages its own local immutable parts. RustHouse rejects overlapping part ranges, inconsistent schemas, and snapshots with more than 10,000 parts. Primary-index predicate pruning is not implemented.

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

# RustHouse

**RustHouse is an experimental Rust SQL CLI and offline MergeTree inspector.** It queries generated numbers or frozen wide parts using a limited, read-only SELECT dialect. It also has a separate metadata-only schema catalog and a localhost HTTP server. **Not a production database.**

## Quick start

Requires **Rust 1.85+**. Run from the repository root:

```sh
cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10) WHERE number >= 7 ORDER BY number DESC LIMIT 3"
cargo run -p rusthouse-cli -- sql "SELECT (2 + 3) * 4 AS result"
```

Output:

```text
number
9
8
7
```

## Bounded SQL joins

Two persistent tables can be queried with a typed, in-memory INNER equi-join:

```sh
cargo run -p rusthouse-cli -- sql "SELECT u.id, t.label FROM users u JOIN teams t ON u.team_id = t.id ORDER BY u.id LIMIT 10" --database ./mydb
```

Join columns must use qualified names. The current join executor supports two named tables, up to eight equi-keys connected by AND, INNER/LEFT/RIGHT/FULL [OUTER] JOIN, optional aliases, \`SELECT *\`, ORDER BY, LIMIT and OFFSET. NULL join keys do not match. Scans are limited to 4,096 rows per table and results to 65,536 rows; larger queries fail explicitly.

## Query frozen parts

```sh
cargo run -p rusthouse-cli -- inspect-part /path/to/wide-part --json
cargo run -p rusthouse-cli -- sql "SELECT id, count(*) FROM part GROUP BY id ORDER BY id" --part /path/to/frozen-active-parts
```

`--part` accepts one wide part or a directory of compatible active wide parts. `part` is the SQL source for those files, not a catalog table.

## Supported features

- **SQL:** `SELECT * RENAME old AS new` (query output only), `WHERE` (including `IN`, `BETWEEN`, `LIKE`, and `ILIKE`), sorting (including `ORDER BY ... NULLS FIRST/LAST`), `DISTINCT`, numeric aggregates (including `median`), grouped numeric aggregates by one key, `COUNT(column)` (skips NULLs), grouped counts with `HAVING`, and pagination. Scalar `SELECT` without `FROM` also supports signed integer arithmetic (`+`, `-`, `*`, `%`) with parentheses, checked 64-bit overflow and zero-divisor errors. This is a limited SQL subset.
- **Data:** `numbers()` and inclusive `generate_series(start, stop[, step])` without files, plus supported numeric, `String`, `UUID`, and `Nullable` part columns.
- **Output:** TSV, CSV, JSON, JSON Lines, Markdown, HTML, and vertical formats via `--format`; also `--output`, `--max-rows`, `--time`, `--show-ast`, `--explain`, `--file`, and `--stdin`.
- **Inspection:** `verify-part`, `verify-file-sizes`, `inspect-marks`, `inspect-checksums`, `inspect-blocks`, and `inspect-primary-index`. Run `cargo run -p rusthouse-cli -- --help` for the full CLI.
- **HTTP:** an optional [read-only localhost server](crates/rusthouse-server/README.md), with no authentication or remote access.

## Local SQL database (experimental)

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, name String)" --database ./mydb
cargo run -p rusthouse-cli -- sql "INSERT INTO events VALUES (1, 'Ada'), (2, 'Grace')" --database ./mydb
cargo run -p rusthouse-cli -- sql "SELECT id, name FROM events ORDER BY id" --database ./mydb
cargo run -p rusthouse-cli -- sql "CREATE TABLE generated (id UInt64)" --database ./mydb
cargo run -p rusthouse-cli -- sql "INSERT INTO generated (id) SELECT number FROM numbers(5)" --database ./mydb
cargo run -p rusthouse-cli -- sql "COPY events FROM '/path/to/events.csv' WITH (FORMAT CSV, HEADER TRUE)" --database ./mydb
```

COPY FROM CSV imports local UTF-8 CSV with quoted commas, quotes, and newlines. With `HEADER TRUE`, CSV headers map fields to named table columns. Without a header, CSV fields follow table column order. Unquoted empty fields become NULL; quoted empty fields remain empty strings. Imports are limited to 4,096 records and 8 MiB and publish one part only after parsing succeeds.

INSERT ... SELECT reads up to 4,096 rows from another table or a supported generated source and appends them as one part. It supports named target columns and the same type checks as INSERT VALUES. A SELECT that yields no rows is a no-op. The read and write are separate operations, **not a transaction**.

`--database` stores table schemas and **immutable wide parts** locally. Supported SQL includes `CREATE TABLE`, `INSERT ... VALUES`, `SELECT`, `SHOW TABLES`, `SHOW CREATE TABLE`, `DESCRIBE TABLE`, `CREATE TABLE ... LIKE`, `RENAME TABLE`, `DROP TABLE` and `TRUNCATE TABLE`. Queries support the existing `--format`, `--output`, `--max-rows` and `--explain` options. Database writes are single-writer, limited to supported numeric, String and Nullable columns, and **not transactional across statements**. This is for local experiments, not concurrent production workloads. Run a sequence of up to 128 statements (256 KiB total) with `rusthouse sql --script setup.sql --database ./mydb`. Scripts permit SQL line comments and stop on the first error; they are **not transactions**.

**Current limits:** SQL statements are limited to 4 KiB, INSERT supports up to 4,096 rows per statement, and tables are limited to 10,000 immutable parts. There is no JOIN, background compaction, or cross-statement transaction. On Unix, the persistent `.rusthouse-write.lock` file uses a kernel advisory lock, automatically released when a process exits or crashes. Do not delete this file while the database is in use. Stop older RustHouse processes before upgrading. Other platforms retain create-new lock-file semantics. The standalone HTTP server does not participate in the CLI database lock protocol.

`DELETE FROM events WHERE id = 1` removes matching rows from the active snapshot, preserving the previous generation for recovery. `DELETE FROM events` removes all rows, while `TRUNCATE TABLE events` is the faster option when there is no predicate. DELETE shares the 25,000-row and 16 MiB rewrite limits.

Use `UPDATE events SET message = 'fixed' WHERE id = 2` to replace literal values in matching rows. Multiple assignments and `NULL` are supported when the target column allows NULL. UPDATE uses the same bounded copy-on-write snapshots as DELETE and OPTIMIZE; expressions such as `SET count = count + 1` are not supported yet.

## Reclaim active parts safely

`OPTIMIZE TABLE events` rewrites a bounded local table into new immutable parts and switches its active snapshot with an atomic pointer replacement. Previous snapshots are retained for existing readers and recovery. A rewrite supports at most 25,000 rows and 16 MiB of materialized values; use this only for local experiments.

```sh
cargo run -p rusthouse-cli -- sql "OPTIMIZE TABLE events" --database ./mydb
```

## Local schema catalog

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, name String)" --catalog ./catalog
cargo run -p rusthouse-cli -- sql "SHOW TABLES" --catalog ./catalog
```

`CREATE`, `ALTER`, `RENAME`, `DROP`, `DESCRIBE`, and `SHOW CREATE TABLE` manage **local schema metadata only**. They do not modify MergeTree data, and catalog tables cannot be queried with `SELECT`.

## Safety and limits

**Only use frozen, offline snapshots.** Never read a live table directory while another process modifies it. RustHouse rejects overlapping part ranges, inconsistent schemas, and snapshots with more than 10,000 parts. Primary-index predicate pruning is not implemented.

## Benchmarks

Run the in-memory SQL execution benchmark in release mode:

```sh
cargo bench -p rusthouse-query --bench queries --locked -- --rows 100000 --samples 11
```

It measures filtered counts, filtered sums, and filtered top-K queries over generated `numbers()` rows. Each query is parsed before timing, warmed up three times, and checked for the expected result. The output shows median latency and millions of input rows per second.

Measure filtered scans on locally written immutable wide parts in release mode:

```sh
cargo bench -p rusthouse-query --bench parts --locked -- --rows 65536 --samples 11
```

The disk benchmark generates its parts once before timing, parses SQL before samples, warms each query three times, and checks query results. This includes filesystem reads with OS caching but does not force cold-cache operation. It is not a ClickHouse comparison.

This covers query execution and formatting into a memory buffer, **not** disk scans, terminal rendering, or server overhead. Compare runs on the same machine and Rust compiler version; shared CI runners are not suitable for stable performance numbers.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

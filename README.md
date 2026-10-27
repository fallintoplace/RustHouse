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

Join columns must use qualified names. The current join executor supports two named tables, up to eight equi-keys connected by AND, INNER/LEFT/RIGHT/FULL [OUTER] JOIN, optional aliases, \`SELECT *\`, ORDER BY, LIMIT and OFFSET. NULL join keys do not match. Unsorted INNER JOIN streams up to 100,000 rows on the left and builds up to 32,768 right-side rows (with a 32 MiB estimated retained-cell budget). Sorted INNER joins and outer joins support up to 16,384 rows per table, subject to the 32 MiB retained-row budget per side. All joins reject more than 65,536 matched output rows.

## Query frozen parts

```sh
cargo run -p rusthouse-cli -- inspect-part /path/to/wide-part --json
cargo run -p rusthouse-cli -- sql "SELECT id, count(*) FROM part GROUP BY id ORDER BY id" --part /path/to/frozen-active-parts
```

`--part` accepts one wide part or a directory of compatible active wide parts. `part` is the SQL source for those files, not a catalog table.

## Supported features

- **SQL:** `SELECT * RENAME old AS new` (query output only), `WHERE` (including `IN`, `BETWEEN`, `LIKE`, and `ILIKE`), sorting (including `ORDER BY ... NULLS FIRST/LAST`), `DISTINCT`, numeric aggregates (including `median`), grouped numeric aggregates by one key, `COUNT(column)` (skips NULLs), grouped counts with `HAVING`, and pagination. Scalar `SELECT` without `FROM` also supports signed integer arithmetic (`+`, `-`, `*`, `%`) with parentheses, checked 64-bit overflow and zero-divisor errors, plus `GREATEST` and `LEAST` with 2 to 16 integer expressions. This is a limited SQL subset.
- **Scalar integers:** `CLAMP(value, lower, upper)` limits an evaluated integer expression to inclusive bounds in `SELECT` without `FROM`. Reversed bounds are rejected.
- **Integer arithmetic:** Scalar `GCD(a, b)` and `LCM(a, b)` work on signed constants, return non-negative signed results, and report overflow when a magnitude exceeds `Int64`.
- **Bit shifts:** Scalar `bitShiftLeft(value, amount)` and `bitShiftRight(value, amount)` accept shift counts from 0 to 63, reject overflowing left shifts, and perform arithmetic right shifts on signed integers.
- **Rounded division:** Scalar `CEIL_DIV(a, b)` and `FLOOR_DIV(a, b)` round the signed integer quotient toward positive or negative infinity. Zero divisors and signed 64-bit overflow are rejected.
- **Data:** `numbers()` and inclusive `generate_series(start, stop[, step])` without files, plus supported numeric, `String`, `UUID`, and `Nullable` part columns.
- **Output:** TSV, CSV, JSON, JSON Lines, RFC 7464 JSON text sequences (`--format json-seq`), compact JSON row arrays (`--format json-compact-each-row`), Markdown, HTML, and vertical formats via `--format`; also `--output`, `--max-rows`, `--time`, `--show-ast`, `--explain`, `--file`, and `--stdin`.
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

COPY FROM CSV imports local UTF-8 CSV with quoted commas, quotes, and newlines. With `HEADER TRUE`, CSV headers map fields to named table columns. Without a header, CSV fields follow table column order. Unquoted empty fields become NULL; quoted empty fields remain empty strings. CSV imports are limited to 50,000 data rows and 8 MiB. Imports above 4,096 rows publish multiple verified parts by atomically switching the active snapshot. Existing immutable parts are preserved. TSV imports now support the same 50,000-row and 8 MiB limits as CSV, with atomic multi-part publication.

INSERT ... SELECT reads up to 50,000 rows (8 MiB of TSV output) from another table or a supported generated source and publishes the rows in one or more parts atomically. It supports named target columns and the same type checks as INSERT VALUES. A SELECT that yields no rows is a no-op. The source read and append hold a single exclusive database lock, preventing concurrent table changes during the statement. This is **not** a cross-statement transaction.

`--database` stores table schemas and **immutable wide parts** locally. Supported SQL includes `CREATE TABLE`, `INSERT ... VALUES`, `SELECT`, `SHOW TABLES`, `SHOW CREATE TABLE`, `DESCRIBE TABLE`, `CREATE TABLE ... LIKE`, `RENAME TABLE`, `DROP TABLE` and `TRUNCATE TABLE`. Queries support the existing `--format`, `--output`, `--max-rows` and `--explain` options. Database writes are single-writer, limited to supported numeric, String and Nullable columns, and **not transactional across statements**. This is for local experiments, not concurrent production workloads. Run a sequence of up to 128 statements (256 KiB total) with `rusthouse sql --script setup.sql --database ./mydb`. Scripts permit SQL line comments and stop on the first error; they are **not transactions**.

**Current limits:** SQL statements are limited to 4 KiB, INSERT supports up to 4,096 rows per statement, and tables are limited to 10,000 immutable parts. There is no JOIN, background compaction, or cross-statement transaction. On Unix, the persistent `.rusthouse-write.lock` file uses a kernel advisory lock, automatically released when a process exits or crashes. Do not delete this file while the database is in use. Stop older RustHouse processes before upgrading. Other platforms retain create-new lock-file semantics. The localhost HTTP server shares the CLI database read lock for persistent table queries.

`DELETE FROM events WHERE id = 1` removes matching rows from the active snapshot, preserving the previous generation for recovery. `DELETE FROM events` removes all rows, while `TRUNCATE TABLE events` is the faster option when there is no predicate. DELETE shares the 25,000-row and 16 MiB rewrite limits.

Use `UPDATE events SET message = 'fixed' WHERE id = 2` to replace literal values in matching rows. Multiple assignments and `NULL` are supported when the target column allows NULL. UPDATE uses the same bounded copy-on-write snapshot publication as DELETE; expressions such as `SET count = count + 1` are not supported yet.

## Reclaim active parts safely

`OPTIMIZE TABLE events` rewrites a bounded local table into new immutable parts and switches its active snapshot with an atomic pointer replacement. Previous snapshots are retained for existing readers and recovery. OPTIMIZE streams rows in batches of at most 4,096 to verified immutable parts, then atomically switches the active snapshot. UPDATE and DELETE still use the 25,000-row / 16 MiB rewrite limits. Use only for local experiments.

```sh
cargo run -p rusthouse-cli -- sql "OPTIMIZE TABLE events" --database ./mydb
```

## Local schema catalog

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, name String)" --catalog ./catalog
cargo run -p rusthouse-cli -- sql "SHOW TABLES" --catalog ./catalog
```

`CREATE`, `ALTER`, `RENAME`, `DROP`, `DESCRIBE`, and `SHOW CREATE TABLE` manage **local schema metadata only**. `SHOW TABLES NOT LIKE 'pattern'` excludes matching schema names and supports the same `%`, `_`, and backslash escapes as `SHOW TABLES LIKE`. `SHOW COLUMNS FROM events` and `DESC TABLE events` are aliases for `DESCRIBE TABLE events` in both catalog and database mode. They do not modify MergeTree data, and catalog tables cannot be queried with `SELECT`.

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

## Diagnose a local database without changing it

Stop writers and the HTTP server before running the offline diagnostic command:

```sh
rusthouse doctor --database ./mydb
```

It checks catalog schemas against active parts, validates stream checksums,
mark layouts, and manifest file sizes, and reports missing or orphaned tables,
bad snapshot pointers, and interrupted staging paths. Reports are sorted by
table name. Errors produce a nonzero exit code. The command does not delete,
repair, or create database files, including the lock file.

Run it only with writers stopped: doctor does not take a database lock.
It does **not** verify checksums.txt file hashes, older inactive generations,
or power-loss durability. Preserve damaged files before manual recovery.

## Debug-only storage crash testing

Debug builds provide internal process-exit fault points around INSERT and
snapshot publication. These are disabled in release builds and must not be used
for a running database. The test command below launches disposable databases,
forces process exit 86 at each selected point, reopens storage, and checks
committed rows and subsequent writes:

```sh
cargo test -p rusthouse-cli --test sql_database_crash_points --locked
```

This models sudden **process termination**, not power loss or a filesystem
that violates flush guarantees. Ordinary tests do not set the fault-point
environment variable.

## License

RustHouse is licensed under the [Apache License, Version 2.0](LICENSE).

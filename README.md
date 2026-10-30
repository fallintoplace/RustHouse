# RustHouse

**RustHouse is an experimental Rust SQL CLI and offline MergeTree inspector.** It queries generated numbers or frozen wide parts using a limited, read-only SELECT dialect. It also has a separate metadata-only schema catalog and a localhost HTTP server. **Not a production database.**

The five-crate Cargo workspace separates storage, query execution, shared result formatting, the CLI, and the HTTP server. `rusthouse-format` contains output adapters used by both entry points.

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

Join columns must use qualified names. NULL join keys do not match. The parser supports up to eight named tables in a JOIN chain, up to eight equality keys per JOIN, INNER/LEFT/RIGHT/FULL joins, optional aliases, projections, ORDER BY, LIMIT and OFFSET.

**Two-table execution limits:** An unordered INNER JOIN can build a hash table from up to **262,144 build-side rows** (subject to a **32 MiB** retained-cell budget) while streaming the other side without a fixed probe-row cap. An ordered or non-streaming outer JOIN normally retains at most **32,768 rows per input**, also subject to the byte budget; specialized ordered and RIGHT OUTER paths can stream larger inputs when applicable. An unbounded two-table JOIN rejects more than **131,072 matched rows**. Supported paged INNER JOINs can scan larger match counts without retaining all output rows; LIMIT and OFFSET still determine the returned page.

**Chained joins:** Three to eight tables currently use a bounded materialized intermediate, with a **64 MiB** retained-row budget and the **131,072 matched rows** limit. This is experimental hash-join execution, not a disk-spilling general-purpose join planner. See `crates/rusthouse-query/src/join.rs` for the shared runtime limits.

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
- **Bit inspection:** Scalar `bitCount(value)`, `bitNot(value)`, and `bitTest(value, position)` inspect or invert 64-bit two's-complement patterns. Bit positions range from 0 to 63.
- **Data:** `numbers()` and inclusive `generate_series(start, stop[, step])` without files, plus supported numeric, `String`, `UUID`, and `Nullable` part columns.
- **Output:** TSV, CSV, JSON, JSON Lines, RFC 7464 JSON text sequences (`--format json-seq`), compact JSON row arrays (`--format json-compact-each-row`), Markdown, HTML, and vertical formats via `--format`; also `--output`, `--max-rows`, `--time`, `--show-ast`, `--explain`, `--file`, and `--stdin`.
- **Inspection:** `verify-part`, `verify-file-sizes`, `inspect-marks`, `inspect-checksums`, `inspect-blocks`, and `inspect-primary-index`. Run `cargo run -p rusthouse-cli -- --help` for the full CLI.
- **HTTP:** an optional [read-only localhost server](crates/rusthouse-server/README.md), without built-in authentication; it binds to loopback by default and permits trusted private IPv4 networking only with explicit opt-in.

## Local SQL database (experimental)

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, name String)" --database ./mydb
cargo run -p rusthouse-cli -- sql "INSERT INTO events VALUES (1, 'Ada'), (2, 'Grace')" --database ./mydb
cargo run -p rusthouse-cli -- sql "SELECT id, name FROM events ORDER BY id" --database ./mydb
cargo run -p rusthouse-cli -- sql "CREATE TABLE generated (id UInt64)" --database ./mydb
cargo run -p rusthouse-cli -- sql "INSERT INTO generated (id) SELECT number FROM numbers(5)" --database ./mydb
cargo run -p rusthouse-cli -- sql "COPY events FROM '/path/to/events.csv' WITH (FORMAT CSV, HEADER TRUE)" --database ./mydb
```

COPY FROM JSONEachRow imports ClickHouse newline-delimited JSON objects with schema-aware field mapping, nullable missing fields, strict JSON escaping, and atomic failure on malformed input. Use `COPY events FROM '/path/to/events.jsonl' WITH (FORMAT JSONEachRow)`. Records are bounded to 1 MiB each, and CSV-style headers are not allowed.

COPY FROM CSV imports local UTF-8 CSV with quoted commas, quotes, and newlines. With `HEADER TRUE`, CSV headers map fields to named table columns. Without a header, CSV fields follow table column order. Unquoted empty fields become NULL; quoted empty fields remain empty strings. CSV imports are limited to 50,000 data rows and 8 MiB. Large imports publish verified parts by atomically switching the active snapshot. Existing immutable parts are preserved. TSV imports stream UTF-8 records instead of buffering the entire file. They support imports larger than 50,000 rows and 8 MiB of total input, with a per-record 8 MiB bound, a bounded part buffer, and the existing 16,384-part limit. CSV and TSV bulk inserts both publish verified parts atomically.

INSERT ... SELECT streams results from another table or a supported generated source to an on-disk spool, then writes verified immutable parts with one atomic snapshot publication. Output is not held in a single unbounded in-memory result. Each decoded output record is limited to 8 MiB. Queries remain subject to the executor's own result and snapshot limits. It supports named target columns and the same type checks as INSERT VALUES. A SELECT that yields no rows is a no-op. The source read and append hold a single exclusive database lock, preventing concurrent table changes during the statement. This is **not** a cross-statement transaction.

`--database` stores table schemas and **immutable wide parts** locally. Supported SQL includes `CREATE TABLE`, `INSERT ... VALUES`, `SELECT`, `SHOW TABLES`, `SHOW CREATE TABLE`, `DESCRIBE TABLE`, `CREATE TABLE ... LIKE`, `RENAME TABLE`, `DROP TABLE` and `TRUNCATE TABLE`. Queries support the existing `--format`, `--output`, `--max-rows` and `--explain` options. Database writes are single-writer, limited to supported numeric, String and Nullable columns, and **not transactional across statements**. This is for local experiments, not concurrent production workloads. Run a sequence of up to 128 statements (256 KiB total) with `rusthouse sql --script setup.sql --database ./mydb`. Scripts permit SQL line comments and stop on the first error; they are **not transactions**.

**Current limits:** CLI SQL input is limited to 64 KiB (the tokenizer also has a token-count bound), INSERT supports up to 4,096 rows per statement, and tables are limited to 16,384 immutable parts. JOIN is supported in the bounded read-only modes described above; background compaction and cross-statement transactions are not supported. On Unix, the persistent `.rusthouse-write.lock` file uses a kernel advisory lock, automatically released when a process exits or crashes. Do not delete this file while the database is in use. Stop older RustHouse processes before upgrading. Other platforms retain create-new lock-file semantics. The localhost HTTP server shares the CLI database read lock for persistent table queries.

`DELETE FROM events WHERE id = 1` removes matching rows from the active snapshot, preserving the previous generation for recovery. `DELETE FROM events` removes all rows, while `TRUNCATE TABLE events` is the faster option when there is no predicate. DELETE streams replacement parts in bounded batches. A single rewritten row is limited to 16 MiB of values; the 16,384-part snapshot limit still applies.

Use `UPDATE events SET message = 'fixed' WHERE id = 2` to replace literal values in matching rows. Multiple assignments and `NULL` are supported when the target column allows NULL. UPDATE uses the same bounded copy-on-write snapshot publication as DELETE; expressions such as `SET count = count + 1` are not supported yet.

## Reclaim active parts safely

`OPTIMIZE TABLE events` rewrites a bounded local table into new immutable parts and switches its active snapshot with an atomic pointer replacement. Previous snapshots are retained for existing readers and recovery. OPTIMIZE batches up to 65,536 rows per verified immutable part (subject to a 4 MiB buffered-value budget), then atomically switches the active snapshot. UPDATE and DELETE also stream copy-on-write replacements, with at most 4,096 rows buffered per part and a 4 MiB normal batch-value budget. Use only for local experiments.

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

**Only use frozen, offline snapshots.** Never read a live table directory while another process modifies it. RustHouse rejects overlapping part ranges, inconsistent schemas, and snapshots with more than 16,384 parts. Primary-index range pruning exists for selected numeric predicates, but is not a general cost-based optimizer.

## Benchmarks

Measure the wall-clock time of 20 verified SQL queries with no external fixture:

```sh
cargo build --release --locked -p rusthouse-cli
python3 -m bench workload latency -- --binary target/release/rusthouse --rows 65536 --runs 5 --warmups 2 --output bench/results/sql-latency.json
```

See the [SQL latency workload](bench/latency/README.md) for timings, correctness checks, and scope.

Run all four deterministic Rust benchmark variants with the [central runner](bench/README.md):

```sh
python3 -m bench benchmark --rows 65536 --samples 7
```

This records a machine/compiler manifest, raw output, and errors in `bench/results/`.
The schedule uses the same command. You can still run individual Cargo benchmarks:

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

Run the full local check suite (Rust formatting, Clippy, tests, build and all Python harness tests):

```sh
python3 -m bench check
```

Individual Rust commands remain available:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## Diagnose a local database without changing it

Stop writers and the HTTP server before running the offline diagnostic command:

```sh
rusthouse doctor --database ./mydb
rusthouse doctor --database ./mydb --full-hashes
```

It checks catalog schemas against active parts, validates stream checksums,
mark layouts, and manifest file sizes, and reports missing or orphaned tables,
bad snapshot pointers, and interrupted staging paths. Reports are sorted by
table name. Errors produce a nonzero exit code. The command does not delete,
repair, or create database files, including the lock file.

Run it only with writers stopped: doctor does not take a database lock.
By default, doctor checks block checksums and manifest file sizes only.
With `--full-hashes`, it also reads every active manifest-listed file and
verifies its **stored-byte** CityHash128WithSeed checksum. This can detect
same-size corruption in metadata, marks, and compressed streams, but may take
significant time and disk I/O on large snapshots. Uncompressed manifest hashes,
older inactive generations, and power-loss durability are not verified.
Preserve damaged files before manual recovery.

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

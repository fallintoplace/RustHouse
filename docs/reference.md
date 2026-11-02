# RustHouse reference

This guide covers SQL features, local storage, limits, and benchmarks.

## Bounded SQL joins

Join two local tables with an INNER JOIN:

```sh
cargo run -p rusthouse-cli -- sql "SELECT u.id, t.label FROM users u JOIN teams t ON u.team_id = t.id ORDER BY u.id LIMIT 10" --database ./mydb
```

Use qualified names for join columns, such as `u.team_id`. NULL join keys do not match. RustHouse supports INNER, LEFT, RIGHT, and FULL joins, with up to eight tables and eight equality keys per join. Aliases, projections, ORDER BY, LIMIT, and OFFSET are also supported.

**Limits for two-table joins:**

- An unordered INNER JOIN can build a hash table with up to **262,144 rows** and **32 MiB** of retained cells. It can stream the other input without a fixed row limit.
- An ordered or non-streaming outer JOIN normally retains up to **32,768 rows per input**, also within the byte limit. Some ordered and RIGHT OUTER paths can stream larger inputs.
- An unbounded two-table JOIN rejects more than **131,072 matched rows**. Supported paged INNER JOINs can scan more matches without retaining every output row. LIMIT and OFFSET select the returned page.

**Joins with three to eight tables** keep intermediate rows in memory. They have a **64 MiB** retained-row budget and a **131,072 matched-row** limit. This experimental hash join does not spill to disk. See `crates/rusthouse-query/src/join.rs` for the shared limits.

## Query frozen parts

```sh
cargo run -p rusthouse-cli -- inspect-part /path/to/wide-part --json
cargo run -p rusthouse-cli -- sql "SELECT id, count(*) FROM part GROUP BY id ORDER BY id" --part /path/to/frozen-active-parts
```

Pass one wide part or a folder of compatible active wide parts to `--part`. In SQL, use `part` as the source name.

## Supported features

- **SQL:** `SELECT * RENAME old AS new` (query output only), `WHERE` (including `IN`, `BETWEEN`, `LIKE`, and `ILIKE`), sorting (including `ORDER BY ... NULLS FIRST/LAST`), `DISTINCT`, numeric aggregates (including `median`), grouped aggregates, multi-key grouped counts, `COUNT(column)` (skips NULLs), `HAVING`, and pagination. Scalar `SELECT` without `FROM` also supports signed integer arithmetic (`+`, `-`, `*`, `%`) with parentheses, checked 64-bit overflow and zero-divisor errors, plus `GREATEST` and `LEAST` with 2 to 16 integer expressions.
- **Scalar integers:** `CLAMP(value, lower, upper)` limits an evaluated integer expression to inclusive bounds in `SELECT` without `FROM`. Reversed bounds are rejected.
- **Integer arithmetic:** Scalar `GCD(a, b)` and `LCM(a, b)` work on signed constants, return non-negative signed results, and report overflow when a magnitude exceeds `Int64`.
- **Bit shifts:** Scalar `bitShiftLeft(value, amount)` and `bitShiftRight(value, amount)` accept shift counts from 0 to 63, reject overflowing left shifts, and perform arithmetic right shifts on signed integers.
- **Rounded division:** Scalar `CEIL_DIV(a, b)` and `FLOOR_DIV(a, b)` round the signed integer quotient toward positive or negative infinity. Zero divisors and signed 64-bit overflow are rejected.
- **Bit inspection:** Scalar `bitCount(value)`, `bitNot(value)`, and `bitTest(value, position)` inspect or invert 64-bit two's-complement patterns. Bit positions range from 0 to 63.
- **Data:** `numbers()` and inclusive `generate_series(start, stop[, step])` without files, plus supported numeric, `String`, `UUID`, and `Nullable` part columns.
- **Output:** TSV, CSV, JSON, JSON Lines, RFC 7464 JSON text sequences (`--format json-seq`), compact JSON row arrays (`--format json-compact-each-row`), Markdown, HTML, and vertical formats via `--format`; also `--output`, `--max-rows`, `--time`, `--show-ast`, `--explain`, `--file`, and `--stdin`.
- **Inspection:** `verify-part`, `verify-file-sizes`, `inspect-marks`, `inspect-checksums`, `inspect-blocks`, and `inspect-primary-index`. Run `cargo run -p rusthouse-cli -- --help` for the full CLI.
- **HTTP:** an optional [read-only localhost server](../crates/rusthouse-server/README.md), without built-in authentication; it binds to loopback by default and permits trusted private IPv4 networking only with explicit opt-in.

## Local SQL database (experimental)

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, name String)" --database ./mydb
cargo run -p rusthouse-cli -- sql "INSERT INTO events VALUES (1, 'Ada'), (2, 'Grace')" --database ./mydb
cargo run -p rusthouse-cli -- sql "SELECT id, name FROM events ORDER BY id" --database ./mydb
cargo run -p rusthouse-cli -- sql "CREATE TABLE generated (id UInt64)" --database ./mydb
cargo run -p rusthouse-cli -- sql "INSERT INTO generated (id) SELECT number FROM numbers(5)" --database ./mydb
cargo run -p rusthouse-cli -- sql "COPY events FROM '/path/to/events.csv' WITH (FORMAT CSV, HEADER TRUE)" --database ./mydb
```

**JSONEachRow:** Use `COPY events FROM '/path/to/events.jsonl' WITH (FORMAT JSONEachRow)` to import newline-delimited JSON objects. Fields are mapped to the table schema. Missing nullable fields are allowed, JSON escapes are checked, and malformed input fails atomically. Each record is limited to 1 MiB. CSV-style headers are not allowed.

**CSV:** Import local UTF-8 files, including quoted commas, quotes, and newlines. With `HEADER TRUE`, header names map to table columns. Without a header, fields follow table column order. An unquoted empty field becomes NULL; a quoted empty field stays an empty string. CSV imports are limited to 50,000 data rows and 8 MiB.

**TSV:** Imports stream UTF-8 records, so the whole file is not held in memory. TSV supports more than 50,000 rows and more than 8 MiB of input. Each record is limited to 8 MiB. Part buffering is bounded, and the 16,384-part limit applies.

CSV and TSV bulk imports publish verified parts by switching the active snapshot atomically. Existing immutable parts are preserved.

**INSERT ... SELECT:** Results from a table or supported generated source are first written to a disk spool. RustHouse then writes verified immutable parts and switches the active snapshot atomically. It does not keep the full output in memory. Each decoded record is limited to 8 MiB, and query result and snapshot limits still apply.

You can name target columns. Type checks match INSERT VALUES. A SELECT with no rows makes no changes. One exclusive database lock covers the source read and append, so tables cannot change during the statement. This is **not** a transaction across multiple statements.

`--database` stores table schemas and **immutable wide parts** locally. Supported commands include `CREATE TABLE`, `INSERT ... VALUES`, `SELECT`, `SHOW TABLES`, `SHOW CREATE TABLE`, `DESCRIBE TABLE`, `CREATE TABLE ... LIKE`, `RENAME TABLE`, `DROP TABLE`, and `TRUNCATE TABLE`. Queries support `--format`, `--output`, `--max-rows`, and `--explain`.

Local databases are for experiments. Writes use a single writer and support selected numeric, String, and Nullable columns. Multiple statements are **not** one transaction. To run up to 128 statements (256 KiB total), use `rusthouse sql --script setup.sql --database ./mydb`. Scripts allow SQL line comments and stop at the first error.

**Current limits:** SQL input is limited to 64 KiB and a bounded number of tokens. INSERT supports up to 4,096 rows per statement. A table can have up to 16,384 immutable parts. Joins use the limits described above. Background compaction and transactions across statements are not supported.

On Unix, the persistent `.rusthouse-write.lock` uses a kernel advisory lock. The lock is released when a process exits or crashes. Do not delete the file while the database is in use. Stop older RustHouse processes before upgrading. Other platforms use create-new lock-file semantics. The HTTP server uses the same database read lock as the CLI for persistent table queries.

`DELETE FROM events WHERE id = 1` removes matching rows from the active snapshot, preserving the previous generation for recovery. `DELETE FROM events` removes all rows, while `TRUNCATE TABLE events` is the faster option when there is no predicate. DELETE streams replacement parts in bounded batches. A single rewritten row is limited to 16 MiB of values; the 16,384-part snapshot limit still applies.

Use `UPDATE events SET name = 'fixed' WHERE id = 2` to replace literal values in matching rows. Multiple assignments and `NULL` are supported when the target column allows NULL. UPDATE uses the same bounded copy-on-write snapshot publication as DELETE; expressions such as `SET count = count + 1` are not supported yet.

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

RustHouse includes reference-checked analytics workloads for local measurement.
On an Apple M1 Pro with 32 GB RAM, a release build at `7c450104` with Rust
1.96.0 measured these median CLI query times on deterministic data with a warm
OS cache:

| Workload | Rows | Median |
| --- | ---: | ---: |
| Counts grouped by region and category | 200,000 events | 21.8 ms |
| Counts grouped by user and ranked | 200,000 events, 997 users | 12.0 ms |
| Four-table inner join, ordered, with `LIMIT 20` | 16,000 events, 997 accounts | 41.6 ms |

Each result matched SQLite over the same data. Measurements use seven runs after
two warmups and include CLI startup, query execution, and TSV output. Fixture
generation, import, and SQLite evaluation are outside the timed samples. These
are synthetic single-query measurements, not a cross-engine comparison. Run
the same workloads on your machine:

```sh
cargo build --release --locked -p rusthouse-cli
python3 -m bench workload analytics-suite -- --binary target/release/rusthouse --workload bench/analytics/workloads/multidimensional-grouping.json --rows 200000 --runs 7 --warmups 2
python3 -m bench workload analytics-suite -- --binary target/release/rusthouse --workload bench/analytics/workloads/group-cardinality-pressure.json --rows 200000 --runs 7 --warmups 2
python3 -m bench workload analytics-suite -- --binary target/release/rusthouse --workload bench/analytics/workloads/join-four-table-chain.json --rows 16000 --runs 7 --warmups 2
```

See the [analytics suite guide](../bench/analytics/SUITES.md) for more workloads.

Measure the wall-clock time of 20 verified SQL queries with no external fixture:

```sh
cargo build --release --locked -p rusthouse-cli
python3 -m bench workload latency -- --binary target/release/rusthouse --rows 65536 --runs 5 --warmups 2 --output bench/results/sql-latency.json
```

See the [SQL latency workload](../bench/latency/README.md) for timings, correctness checks, and scope.

Run all four deterministic Rust benchmark variants with the [central runner](../bench/README.md):

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

The part benchmark creates its parts before timing. It parses each query, warms it up three times, and checks the result. Times include filesystem reads with OS caching. They are not cold-cache results or a ClickHouse comparison.

The in-memory benchmark covers query execution and formatting into a memory buffer. The part benchmark also reads locally written parts. Compare runs on the same machine and Rust compiler version; shared CI runners are not suitable for stable performance numbers.

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

This test models sudden **process termination**. It does not model power loss
or a filesystem that breaks flush guarantees. Ordinary tests do not enable the
fault points.

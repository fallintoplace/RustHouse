# RustHouse

**RustHouse is an experimental Rust SQL CLI and offline MergeTree inspector.** It queries generated numbers or frozen wide parts using a limited, read-only SELECT dialect. It also has a separate metadata-only schema catalog and a localhost HTTP server. **Not a production database.**

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

- **SQL:** `WHERE` (including `IN`, `BETWEEN`, `LIKE`, and `ILIKE`), sorting, `DISTINCT`, numeric aggregates (including `median`), grouped counts with `HAVING`, and pagination. This is a limited SQL subset.
- **Data:** `numbers()` without files, plus supported numeric, `String`, `UUID`, and `Nullable` part columns.
- **Output:** TSV, CSV, JSON, JSON Lines, Markdown, and vertical formats via `--format`; also `--output`, `--max-rows`, `--time`, `--show-ast`, `--explain`, `--file`, and `--stdin`.
- **Inspection:** `verify-part`, `verify-file-sizes`, `inspect-marks`, `inspect-checksums`, `inspect-blocks`, and `inspect-primary-index`. Run `cargo run -p rusthouse-cli -- --help` for the full CLI.
- **HTTP:** an optional [read-only localhost server](crates/rusthouse-server/README.md), with no authentication or remote access.

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

This covers query execution and formatting into a memory buffer, **not** disk scans, terminal rendering, or server overhead. Compare runs on the same machine and Rust compiler version; shared CI runners are not suitable for stable performance numbers.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

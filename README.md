# RustHouse

Experimental read-only MergeTree inspector and local numeric SQL queries. Run `cargo run -p rusthouse-cli -- sql "SELECT 1"` or `cargo run -p rusthouse-cli -- sql "SELECT id FROM part LIMIT 10" --part /path/to/wide-part`.

Try `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10) WHERE number >= 7 LIMIT 3"` without a part.

Sort rows with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10) ORDER BY number DESC LIMIT 3"`.

Aggregate numeric values with `cargo run -p rusthouse-cli -- sql "SELECT sum(number) FROM numbers(1000) WHERE number >= 900"`.

Page results with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(100) ORDER BY number DESC LIMIT 5 OFFSET 10"`.

Combine filters with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(20) WHERE number >= 5 AND number < 12 ORDER BY number DESC"`.

Filter inclusive numeric ranges with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(20) WHERE number BETWEEN 5 AND 12 ORDER BY number DESC LIMIT 3"`.

Filter chosen values with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10) WHERE number IN (1, 3, 7)"`.

Deduplicate numeric rows with `cargo run -p rusthouse-cli -- sql "SELECT DISTINCT id FROM part ORDER BY id" --part /path/to/wide-part`.

Count occurrences of numeric keys with `cargo run -p rusthouse-cli -- sql "SELECT id, count(*) FROM part GROUP BY id ORDER BY id DESC" --part /path/to/wide-part`.

Filter alternatives with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10) WHERE number = 1 OR number >= 8"`.

Filter String columns from a frozen part with `rusthouse sql "SELECT service FROM part WHERE service LIKE 'api%'" --part ./snapshot`. `LIKE` accepts `%` for zero or more bytes and `_` for one byte; `NOT LIKE` excludes matching values. Backslash escapes the next pattern byte. Nulls do not match either predicate.

Start at a chosen number with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(100, 5)"`.

Execute SQL from a saved file with `cargo run -p rusthouse-cli -- sql --file query.sql` (up to 4096 bytes).

Export numeric query results as CSV with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10)" --format csv`.

Export numeric rows as JSON Lines with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10)" --format jsonl`. JSONL emits values as strings to preserve 128-bit integer precision.

Inspect the parsed SQL structure without touching data using `cargo run -p rusthouse-cli -- sql --show-ast "SELECT id FROM part WHERE id >= 3"`.

Exclude selected values with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10) WHERE number NOT IN (1, 4, 9)"`.

Export data-only TSV without the header row with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(3)" --format tsv-no-header`.

Exclude selected remainders with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10) WHERE number % 2 != 0"`.

Name single-column query results with `cargo run -p rusthouse-cli -- sql "SELECT sum(number) AS total FROM numbers(10)"`. Aliases also work in `ORDER BY` for numeric row projections.

Inspect fixed-width primary-index entries with `cargo run -p rusthouse-cli -- inspect-primary-index /path/to/primary.idx 8 4`. Field widths are byte counts. RustHouse prints a bounded hex preview (16 marks, 8 fields, 16 bytes per field) and accepts both `.idx` and `.cidx`.

Negate a numeric predicate using `WHERE NOT number < 5`, `WHERE NOT number IN (1, 3)`, or `WHERE NOT number BETWEEN 2 AND 4`. The prefix `NOT` applies to one predicate; parenthesized Boolean expressions are not supported.

Measure SQL execution and output formatting time with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(1000)" --time`. The elapsed milliseconds appear on stderr, leaving stdout and exported files unchanged.

Inspect part metadata in machine-readable JSON with `cargo run -p rusthouse-cli -- inspect-part /path/to/wide-part --json`. This prints the path, row count, and column names and types without reading data streams.

Copy SQL results into GitHub issues and Markdown documents using `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(3)" --format markdown`. The formatter streams rows and escapes Markdown and HTML-sensitive characters.

## Local schema catalog (experimental)

DDL in RustHouse currently manages **metadata only** in an explicit local directory.
It does **not** create MergeTree parts, change existing data, or make these
catalog tables queryable by `SELECT`. Keep it separate from production parts.

```sh
cargo run -p rusthouse-cli -- sql "CREATE TABLE events (id UInt64, name String)" --catalog ./catalog
cargo run -p rusthouse-cli -- sql "CREATE TABLE IF NOT EXISTS events (id UInt64, name String)" --catalog ./catalog
```

Schemas use a versioned text format in `<catalog>/<table>.schema`.
Identifiers are simple ASCII SQL names and type support is deliberately bounded.

List local tables with `rusthouse sql "SHOW TABLES" --catalog ./catalog`.
Inspect stored column names and types with `rusthouse sql "DESCRIBE TABLE events" --catalog ./catalog`.

Drop only a catalog schema file with `rusthouse sql "DROP TABLE IF EXISTS events" --catalog ./catalog`.
Existing MergeTree parts remain untouched.

Extend a schema with `rusthouse sql "ALTER TABLE events ADD COLUMN age UInt8" --catalog ./catalog`.
`ADD COLUMN IF NOT EXISTS` is idempotent. Changes are metadata-only.

Remove a catalog column with `rusthouse sql "ALTER TABLE events DROP COLUMN age" --catalog ./catalog`.
`DROP COLUMN IF EXISTS` is idempotent. The last column cannot be dropped.

Rename catalog metadata with `rusthouse sql "RENAME TABLE events TO archived_events" --catalog ./catalog`.
The destination must be absent. No MergeTree parts are moved.

Order by a selected column's 1-based position with `SELECT number FROM numbers(10) ORDER BY 1 DESC`. For `SELECT key, count(*) ... GROUP BY key`, positions 1 and 2 refer to the key and count. `SELECT *` requires named ORDER BY columns.

Use SQL-standard pagination with `SELECT number FROM numbers(10) ORDER BY number DESC OFFSET 2 ROWS FETCH NEXT 3 ROWS ONLY`. `FETCH FIRST n ROWS ONLY` also works without an offset; existing `LIMIT` syntax is unchanged.

Rename a column in catalog metadata with `rusthouse sql "ALTER TABLE events RENAME COLUMN name TO title" --catalog ./catalog`. The change preserves column order and never modifies MergeTree data.

Update a local catalog column type with `rusthouse sql "ALTER TABLE events MODIFY COLUMN title Nullable(String)" --catalog ./catalog`. This changes the versioned schema file only; it does not convert existing data, verify its compatibility, or rewrite parts.

Print the parsed logical operator pipeline without scanning data using `rusthouse sql "SELECT number FROM numbers(10) WHERE number > 3 ORDER BY number DESC LIMIT 2" --explain`. This is a logical overview, not a physical or cost-based execution plan; column validity is still checked by actual execution.

Summarize declared part manifest sizes without reading data files with `rusthouse inspect-checksums /path/to/checksums.txt --summary`. The result separates compressed stored bytes and their original byte counts. It does not verify that local files or hashes match the manifest.


## Query frozen multi-part MergeTree snapshots

Pass a directory containing the **active** wide part subdirectories to `--part`:

```sh
rusthouse sql "SELECT count(*) FROM part" --part ./snapshot/active-parts
rusthouse sql "SELECT id, count(*) FROM part GROUP BY id ORDER BY id" --part ./snapshot/active-parts
```

Group mixed-type data with `SELECT service, count(*) FROM part GROUP BY service ORDER BY count DESC`. String, UUID, and Nullable group keys work with WHERE, HAVING, LIMIT and OFFSET. Grouping is bounded to 100,000 distinct keys.

RustHouse now treats either a single wide part or an offline directory of
MergeTree-named active parts as a query source. It streams one part at a time;
aggregates, filtering, ORDER BY, DISTINCT, LIMIT and OFFSET apply to the full set.
For safety, it rejects overlapping block ranges, inconsistent schemas, malformed
parts and more than 10,000 parts. It ignores non-part subdirectories such as
`detached/`. **Only use a frozen snapshot of active parts**, never a live table
directory being modified by a database process. Predicate pruning across the primary index
is not implemented by this change.

Recover portable schema-only DDL with `rusthouse sql "SHOW CREATE TABLE events" --catalog ./catalog`.
The output can be executed in another local catalog. No MergeTree data or engine
settings are included, since the local catalog records only names and types.

Compute medians with `rusthouse sql "SELECT median(number) FROM numbers(100)"`.
`median(column)` works on numeric parts and generated numbers, returns `NULL`
for empty input, and rejects over 100,000 matching rows to bound memory use.
For an odd row count it preserves the middle numeric value; for an even
count it interpolates the two middle values using Float64 precision.

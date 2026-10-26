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

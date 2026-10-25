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

Execute SQL from a saved file with `cargo run -p rusthouse-cli -- sql --file query.sql` (up to 4096 bytes).

Export numeric query results as CSV with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10)" --format csv`.

Export numeric rows as JSON Lines with `cargo run -p rusthouse-cli -- sql "SELECT number FROM numbers(10)" --format jsonl`. JSONL emits values as strings to preserve 128-bit integer precision.

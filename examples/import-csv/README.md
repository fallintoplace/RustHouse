# Import your first CSV

\`\`\`sh
cargo build --locked -p rusthouse-cli
db="$(mktemp -d)"
python3 examples/import-csv/import_csv.py examples/import-csv/purchases.csv --database "$db"
target/debug/rusthouse sql "SELECT region, count(*) FROM trial_purchases GROUP BY region ORDER BY region" --database "$db"
\`\`\`

The importer checks headers, basic integer data, file size and RustHouse's
50,000-row CSV COPY limit. It uses subprocess argument arrays, not a shell,
and escapes file paths in SQL. Only use with a **new local database**: CREATE
and COPY are separate statements and are not atomic together. No network access.

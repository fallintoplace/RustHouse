# RustHouse

Experimental read-only inspector for ClickHouse MergeTree part metadata. It reads `count.txt` and version-1 `columns.txt`. It does not decode column data, verify checksums, or execute SQL.

Requires Rust 1.85+. No external dependencies.

## Run

```sh
cargo run -p rusthouse-cli -- inspect-part crates/rusthouse-storage/tests/fixtures/all_1_1_0
```

## Check

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Next: decode a verified MergeTree wide part, expose Arrow batches for SQL, then add ingestion.

# RustHouse 🦀

An experimental Rust analytical database, inspired by ClickHouse's columnar
storage. **Early prototype, not a production database.**

This first change provides a read-only inspector for the metadata of a local
MergeTree data part. It does **not** execute SQL, decode `.bin` files, validate
checksums, or read rows yet.

## Quick start

Requires Rust 1.85+ (2024 edition). No third-party dependencies are needed for
this first milestone.

```bash
cargo run -p rusthouse-cli -- --help
cargo run -p rusthouse-cli -- inspect-part crates/rusthouse-storage/tests/fixtures/all_1_1_0
```

Example output:

```text
Part: crates/rusthouse-storage/tests/fixtures/all_1_1_0
Rows: 42
Columns (2):
  "id": UInt64
  "city": LowCardinality(String)
```

`inspect-part` accepts a directory from a **local, modern MergeTree part** with
`count.txt` and `columns.txt`. It only reads those metadata files.

## Develop

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The workspace currently has two crates:

- `rusthouse-storage`: parses read-only MergeTree part metadata.
- `rusthouse-cli`: small `rusthouse` executable.

Read the [architecture direction](docs/architecture.md) for the planned Arrow,
DataFusion, and MergeTree reader milestones. Compatibility and performance
claims will only be made after testing against real ClickHouse-generated parts.

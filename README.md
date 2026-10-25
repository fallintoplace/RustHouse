# RustHouse

Experimental read-only ClickHouse MergeTree part metadata and uncompressed wide-part mark inspector. No SQL or column-data decoding.

```sh
cargo run -p rusthouse-cli -- inspect-part crates/rusthouse-storage/tests/fixtures/all_1_1_0
cargo run -p rusthouse-cli -- inspect-marks crates/rusthouse-storage/tests/fixtures/marks/wide.mrk2
```

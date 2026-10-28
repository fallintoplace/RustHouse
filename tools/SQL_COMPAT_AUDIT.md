# Audit SELECT parser compatibility (without reading data)

```sh
cargo build --locked -p rusthouse-cli
python3 tools/sql_compat_audit.py tools/example-queries.sql
cd tools && python3 -m unittest discover -p 'test_sql_compat_audit.py'
```

Each non-comment line must contain one SELECT of at most 4096 UTF-8 bytes.
The tool rejects DDL/DML, multi-statement lines, and input containing other
semicolons. It calls only RustHouse's `--explain` path with a disposable
database directory, producing JSON counts and redacted parser failure text.

**This only checks syntax accepted by RustHouse's current parser.**
An accepted plan is not proof of correct query results, physical execution,
ClickHouse compatibility or acceptable performance. Verify a representative
dataset independently before migration decisions.

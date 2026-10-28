# User-visible persistence acceptance

```sh
cargo build -p rusthouse-cli --locked
python3 -m bench.product_acceptance --binary target/debug/rusthouse
```

Runs CREATE, INSERT, new-process SELECT, exact TSV comparison and read-only
doctor on a disposable directory. Failures return nonzero. The check does
not simulate power loss, transactions, HA, or production concurrency.

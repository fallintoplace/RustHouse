# One-command RustHouse local trial

```sh
bash examples/demo/run.sh
```

Create a temporary database, insert two rows, verify the query results, and run
the read-only offline doctor command. The script cleans up its own temporary
directory. Requires Bash and Rust. For broader evaluation, see the
[first-customer guide](../../docs/customer-evaluation.md).

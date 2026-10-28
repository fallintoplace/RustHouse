# Detect shard schema drift

```sh
python3 tools/schema_diff.py --left http://127.0.0.1:8124 --right http://127.0.0.1:8125 --table events
```

Checks ordered column names and types via the read-only schema endpoint.
It does not compare row counts (shards may legitimately differ), snapshot
generations, data contents or atomic consistency. It only accepts loopback HTTP
servers because the built-in RustHouse server lacks TLS and authentication.

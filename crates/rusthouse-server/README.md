# RustHouse read-only HTTP server

Experimental localhost-only SQL interface. This server is independent of the
RustHouse CLI and runs only the currently supported read-only SELECT dialect.

```sh
cargo run -p rusthouse-server -- --port 8123 --part /path/to/wide-part
curl http://127.0.0.1:8123/ping
curl 'http://127.0.0.1:8123/?query=SELECT%201'
curl http://127.0.0.1:8123/ --data-binary 'SELECT count(*) FROM part'
```

The endpoint supports GET /?query=... and POST / with raw SQL. Responses are TSV.
Inputs are limited to 4096-byte statements and 8192-byte HTTP headers.
Responses are capped at 8 MiB, concurrent requests at 16, and result rows
at 10000 by default (up to 100000 with --max-rows).
Only loopback is supported. There is no TLS, remote access, authentication,
or full wire-protocol compatibility.

For non-generated SQL, use an immutable wide MergeTree part. Multi-part table
snapshots require the separate multi-part query feature.

# RustHouse read-only HTTP server

Experimental localhost-only SQL interface. This server is independent of the
RustHouse CLI and runs only the currently supported read-only SELECT dialect.

```sh
cargo run -p rusthouse-server -- --port 8123 --part /path/to/wide-part
curl http://127.0.0.1:8123/ping
curl http://127.0.0.1:8123/schema
curl 'http://127.0.0.1:8123/?query=SELECT%201'
curl 'http://127.0.0.1:8123/explain?query=SELECT%20number%20FROM%20numbers(10)'
curl http://127.0.0.1:8123/ --data-binary 'SELECT count(*) FROM part'
```

The endpoint supports GET /?query=... and POST / with raw SQL. Responses are TSV by default.
Pass `format=csv` on either method for RFC 4180-style CSV with CRLF row endings.
CSV includes headers, quotes embedded commas/newlines, and uses an unquoted `\\N` for NULL.
Pass `format=jsonl` on either method for `application/x-ndjson`, one JSON object per result row.
Pass `format=json` for a JSON array suitable for API clients. It uses the same validated
JSONL value encoding, an explicit closing bracket, and existing response size limits.
This works with `GET /?query=SELECT%201&format=jsonl` and `POST /?format=jsonl` with a raw SELECT body.
JSONL values are strings to preserve 128-bit numeric precision, and TSV NULL markers become JSON null.
The JSONL converter decodes text escapes, rejects duplicate column names and invalid UTF-8,
and uses the existing 8 MiB response bound plus a 1 MiB per-row bound.
Inputs are limited to 4096-byte statements and 8192-byte HTTP headers.
Responses are capped at 8 MiB, concurrent requests at 16, and result rows
at 10000 by default (up to 100000 with --max-rows).
Only loopback is supported. There is no TLS, remote access, authentication,
or full wire-protocol compatibility.

For non-generated SQL, use an immutable wide MergeTree part. Multi-part table
snapshots require the separate multi-part query feature.

`GET /schema` returns JSON with the frozen part or snapshot's total row count
and an ordered list of column names and types. This endpoint reads metadata
only, requires `--part`, and never exposes DDL or file writes. Schema responses
share the server's 8 MiB response limit.

`GET /explain?query=...` and `POST /explain` with a raw SELECT body
return the bounded logical plan as plain text. The endpoint parses SQL but does
not inspect parts or read rows. It uses the same 4096-byte input limit and
server-side row cap as ordinary queries. This is not a physical or cost-based plan.

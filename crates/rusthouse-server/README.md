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

The endpoint supports GET /?query=... and POST / with raw SQL. A POST may also put the start of a SELECT in `?query=...` and the remaining SQL in its body; the server inserts a newline between nonempty pieces, like ClickHouse. The combined query must still fit within 65536 bytes, and writes remain disabled. Responses are TSV by default.
ClickHouse-compatible clients can use `default_format=TSV` or `default_format=TabSeparated`
for headerless TSV, and `default_format=TSVWithNames` or `TabSeparatedWithNames`
for named columns. `CSV` and `CSVWithNames` select the corresponding CSV modes;
`JSONStringsEachRow` and `JSONCompactStringsEachRow` preserve strings instead of
silently converting them to lossy native JSON numbers. A trailing SQL `FORMAT CSV`, `FORMAT TSV`, `FORMAT TSVWithNames`, or
`FORMAT JSONStringsEachRow` can also select a supported format. Explicit
SQL `FORMAT` takes priority over URL output settings. Unsupported formats
are rejected rather than silently returned with different types.
A legacy `format=...` option takes priority over `default_format`
regardless of URL order.
The existing RustHouse default (named TSV) is unchanged for existing clients.
Pass `format=tsv-no-header` to remove the header row, including for empty results.
Pass `format=csv` on either method for RFC 4180-style CSV with CRLF row endings.
CSV includes headers, quotes embedded commas/newlines, and uses an unquoted `\\N` for NULL.
Use `format=csv-no-header` to omit column headers while preserving CSV escaping and response limits.
Pass `format=sql-values` to export rows as SQL `VALUES ('text', NULL), ...;` tuples.
The output uses quoted text literals for every non-NULL value, matches the CLI format,
and inherits the 16 MiB response and per-line output limits.
Pass `format=markdown` to return a Markdown table with escaped cell text and bounded output.
Pass `format=jsonl` on either method for `application/x-ndjson`, one JSON object per result row.
Pass `format=json-compact-each-row` for newline-delimited JSON arrays of column values.
Like the existing JSONL mode, values remain strings to preserve wide integer
precision instead of emitting ClickHouse's native numeric JSON types.
Pass `format=json-pretty` for an indented JSON array. It preserves the same JSON string and NULL encoding and response bounds.
Pass `format=json` for a JSON array suitable for API clients. It uses the same validated
JSONL value encoding, an explicit closing bracket, and existing response size limits.
This works with `GET /?query=SELECT%201&format=jsonl` and `POST /?format=jsonl` with a raw SELECT body.
JSONL values are strings to preserve 128-bit numeric precision, and TSV NULL markers become JSON null.
The JSONL converter decodes text escapes, rejects duplicate column names and invalid UTF-8,
and uses the existing 16 MiB response bound plus a per-row bound.
Read-only HTTP queries also support typed placeholders such as
`SELECT {min:Int32} + {offset:UInt8}` with `?param_min=2&param_offset=3`.
Supported types are `String`, `Int8/16/32/64`, and `UInt8/16/32/64`;
UInt64 is currently limited to the signed Int64 range by the scalar engine.
Values are range-checked and SQL-escaped before parsing. Missing, duplicate,
unused, or unsupported bindings are rejected. This is a bounded subset of
ClickHouse parameter types and never enables HTTP writes.
ClickHouse's `max_result_rows=N` URL setting can cap individual read-only
requests. It applies to generated queries, persistent tables, JOINs and SQL
metadata discovery. A value of zero means no additional request limit.
Per-request settings never relax the server's configured `--max-rows` cap.

Inputs are limited to 65536-byte statements and 8192-byte HTTP headers.
Responses are capped at 16 MiB, concurrent requests at 16, and result rows
at 10000 by default (up to 250000 with --max-rows).
Loopback is the default. Explicit private IPv4 access is opt-in. There is no TLS or authentication,
or full wire-protocol compatibility.

For non-generated SQL, use an immutable wide MergeTree part or snapshot. To query
persistent tables created by `rusthouse sql ... --database ./mydb`, start the
server with `--database ./mydb` instead of `--part`. Queries like
`SELECT name FROM events` resolve only catalog-visible tables whose schema
matches stored parts. The server remains **read-only**; HTTP DDL and INSERT
are rejected. HTTP database SELECT queries hold the same shared database lock
as CLI readers until execution finishes, so CLI DROP/RENAME cannot remove data
mid-query. Contending writers receive a 409 response while their lock is held. The `--part` and `--database` modes are mutually exclusive.

`GET /schema` returns JSON with the frozen part or snapshot's total row count
and an ordered list of column names and types. This endpoint reads metadata
only, requires `--part`, and never exposes DDL or file writes. With `--database`,
use `GET /schema?table=events` to inspect a named persistent table instead.
Database schema reads use the same shared lock and validation as SQL queries.
Only one URL-encoded `table` parameter is accepted. Schema responses share
the server's 16 MiB response limit.

`GET /explain?query=...` and `POST /explain` with a raw SELECT body
return the bounded logical plan as plain text. The endpoint parses SQL but does
not inspect parts or read rows. It uses the same 65536-byte input limit and
server-side row cap as ordinary queries. This is not a physical or cost-based plan.

In database mode, `DESCRIBE TABLE events` (and `DESC TABLE events`) returns
the seven standard ClickHouse description columns: name, type, default_type,
default_expression, comment, codec_expression, and ttl_expression. The local
catalog has no default expressions, codecs, comments, or TTL definitions, so
those fields are empty. It holds the database read lock and uses the same
schema/active-part validation as SELECT. Supported HTTP formats and row limits
apply. This does not enable schema modifications.

Read-only SQL discovery supports `SHOW TABLES`, `SHOW TABLES LIKE 'event%'`,
and `SHOW TABLES NOT LIKE 'event%'` in database mode. Discovery uses the same
catalog validation and read lock as `GET /tables`. Results respect the HTTP
row cap and selected output format. Other SHOW statements and writes remain disabled.

`GET /capabilities` returns read-only JSON with the server version, source mode,
SQL and response byte limits, and effective row cap. Empty-body GET only.

## Experimental distributed coordinator: explicit loopback peers

The optional `--peer IPv4:<port>` flag registers up to 16 distinct,
read-only peer servers. For example, run independently configured RustHouse
servers on ports 8124 and 8125, then start a coordinator:

```sh
cargo run -p rusthouse-server -- --port 8123 --peer 127.0.0.1:8124 --peer 127.0.0.1:8125
curl http://127.0.0.1:8123/cluster
```

`GET /cluster` lists the explicitly configured addresses. It does not check
whether a peer is alive or whether schemas and datasets agree. Peer addresses default to IPv4 loopback and are never automatically discovered.
Remote peers and listeners require explicit `--allow-private-network`; only
RFC 1918 or 100.64.0.0/10 overlay IPv4 addresses are accepted. No public IPs,
DNS names, or wildcard listeners are allowed.

For cross-host queries, use a trusted encrypted overlay with restrictive ACLs,
such as Tailscale, and bind each shard to its overlay IPv4 address:

```sh
# Run on shard A
rusthouse-server --database ./shard-a --bind 100.101.0.1 --port 8124 --allow-private-network
# Run on shard B
rusthouse-server --database ./shard-b --bind 100.101.0.2 --port 8124 --allow-private-network
# Run the coordinator
rusthouse-server --port 8123 --peer 100.101.0.1:8124 --peer 100.101.0.2:8124 --allow-private-network
```

**Security:** The built-in peer protocol is plaintext HTTP without authentication.
Do not expose it to the public Internet or an untrusted LAN. The overlay must
provide encryption and access control. This adds read-only cross-host queries,
not replication, failover, distributed writes, or atomic multi-shard snapshots.

`GET /cluster/health` checks every configured peer's read-only `/ping`
endpoint in parallel. It returns the number of healthy peers and their addresses
as JSON. An unavailable or invalid peer response makes the HTTP status 503.
The check does not verify data freshness, schema compatibility or consistency.

## Distributed unordered table reads (experimental)

Start two read-only database servers with separately populated shard directories:

```sh
cargo run -p rusthouse-server -- --port 8124 --database ./shard-a --max-rows 100000
cargo run -p rusthouse-server -- --port 8125 --database ./shard-b --max-rows 100000
cargo run -p rusthouse-server -- --port 8123 --peer 127.0.0.1:8124 --peer 127.0.0.1:8125
curl -X POST http://127.0.0.1:8123/distributed --data-binary 'SELECT id, name FROM events'
```

Peer requests use a bounded connect timeout and a total response-read deadline.
Slow trickling peers cannot indefinitely hold a coordinator thread.
The coordinator executes an unordered, read-only table SELECT on each peer,
then concatenates TSV rows in configured peer order with one shared header.
It accepts column projections, `SELECT *`, and pushed-down WHERE clauses;
rejects ORDER BY, DISTINCT, JOIN, and aggregates that require
global semantics. Global LIMIT/OFFSET is supported for unordered scans. The coordinator compares the column names, types, and order from each
peer's table schema before merging. Shards may contain different row counts.
It rejects a shard that reaches its advertised result cap, mismatched headers,
non-200 responses, and coordinator row/byte overflows instead of returning
partial data. LIMIT/OFFSET is applied by the coordinator after validating full shard
responses. Pagination does not bypass shard caps. Schema validation is not an atomic cross-shard transaction, so a concurrent
schema change could still race. This is a snapshot-by-peer scan, **not** an atomic cross-shard
snapshot, automatic partitioning, replication, or remote machine networking.

Distributed grouped counts are supported when all shards contain disjoint rows:

```sql
SELECT category, count(*) FROM events WHERE id >= 10 GROUP BY category
```

The coordinator merges counts for keys shared across shards. Grouped queries
must not include ORDER BY, LIMIT, OFFSET, HAVING or DISTINCT. The result is
bounded by the coordinator's row/byte limits, and a capped shard is rejected
rather than returning incomplete counts. Shard key types must agree.

Distributed `SELECT count(*) FROM events` and
`SELECT count(name) FROM events WHERE ...` are also supported. Each shard
computes its own count; the coordinator adds counts with checked UInt64
arithmetic. Shards must hold **disjoint rows**, otherwise duplicated replicated
rows are counted more than once. `COUNT(DISTINCT ...)` and grouped aggregates
remain unsupported because summing their results would be incorrect.

## Distributed exact integer SUM

Scalar `SUM(column)` can be reduced across disjoint database shards:

```sql
SELECT sum(amount) FROM events WHERE id >= 10
```

The coordinator first verifies that the target column exists on every peer
and has the **same signed or unsigned integer type** (including Nullable).
It adds partial sums using checked 128-bit arithmetic. An empty shard
returns NULL and does not erase a nonempty partial sum. All-empty shards
produce NULL. Mixed types, floating-point sums, overflow, and global
ORDER BY / LIMIT / OFFSET are not supported. Grouped SUM remains unsupported.

SUM requests require an additional schema lookup per shard. As with COUNT,
shards must hold **disjoint source rows** or results double-count data.

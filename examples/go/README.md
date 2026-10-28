# RustHouse Go HTTP client (experimental)

This Go 1.22+ module uses only the Go standard library. It is a read-only
HTTP adapter, **not** a ClickHouse driver.

\`\`\`sh
cd examples/go
go test ./...
\`\`\`

\`\`\`go
db, err := rusthouse.New("http://127.0.0.1:8123")
if err != nil { panic(err) }
rows, err := db.QueryJSON(context.Background(), "SELECT number FROM numbers(3)")
\`\`\`

Numbers are JSON strings to preserve wide integer precision. Query length,
response size and HTTP wait are bounded. Non-loopback hosts are intentionally
rejected because the RustHouse server has no TLS or authentication.

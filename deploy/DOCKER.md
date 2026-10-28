# Rootless RustHouse CLI container

Build from the repository root:

\`\`\`sh
docker build -f deploy/Dockerfile -t rusthouse-cli:local .
docker run --rm rusthouse-cli:local sql "SELECT number FROM numbers(3)"
\`\`\`

For a local persistent trial, create a host-owned directory first:

\`\`\`sh
mkdir -p ./trial-db
docker run --rm --user "$(id -u):$(id -g)" -v "$PWD/trial-db:/data" rusthouse-cli:local sql "CREATE TABLE trial (id UInt64)" --database /data
docker run --rm --user "$(id -u):$(id -g)" -v "$PWD/trial-db:/data" rusthouse-cli:local sql "INSERT INTO trial VALUES (1)" --database /data
docker run --rm -v "$PWD/trial-db:/data:ro" rusthouse-cli:local sql "SELECT * FROM trial" --database /data
\`\`\`

This image intentionally ships **only the CLI**, not a remotely exposed
HTTP server. Persistent writes remain experimental and single-writer.
Use real filesystem permissions; do not share a live database directory
between uncoordinated containers. Bind mounts are not a backup strategy.

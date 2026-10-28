# Check experimental distributed peer health

\`\`\`sh
python3 tools/cluster_health.py --url http://127.0.0.1:8123
\`\`\`

Exits zero only when the server reports internally consistent, fully healthy
peer reachability. It accepts the existing endpoint's HTTP 503 to parse partial
failure details. An empty peer list is valid. **This does not verify replicated
data, snapshot consistency, or query correctness.** Never put the plaintext
RustHouse HTTP port on an untrusted network.

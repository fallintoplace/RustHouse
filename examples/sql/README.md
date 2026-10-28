# Local SQL analytics walkthrough

\`\`\`sh
cargo build --locked -p rusthouse-cli
db="$(mktemp -d)"
target/debug/rusthouse sql --script examples/sql/customer-analytics.sql --database "$db"
target/debug/rusthouse doctor --database "$db"
\`\`\`

Run only against a **new local database**. This demonstrates supported SQL,
not full ClickHouse compatibility. Multi-statement scripts stop at the first
failure but are not atomic across statements. Clean up the trial database
when done.

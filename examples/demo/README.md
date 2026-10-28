# One-command RustHouse local trial

\`\`\`sh
bash examples/demo/run.sh
\`\`\`

Creates a temporary database, inserts two rows, verifies exact query results,
then runs the read-only offline doctor command. The script deletes only its
own temporary directory. Requires Bash and Rust. This does not establish
production durability, crash safety, or ClickHouse compatibility.

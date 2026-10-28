# RustHouse Node.js client (experimental)

Minimal read-only client for Node.js 18+ using built-in \`fetch\`; zero npm dependencies.

\`\`\`sh
node --test examples/javascript/rusthouse-client.test.mjs
cargo run -p rusthouse-server -- --port 8123
\`\`\`

\`\`\`js
import { RustHouseClient } from "./examples/javascript/rusthouse-client.mjs";
const db = new RustHouseClient();
console.log(await db.capabilities());
console.log(await db.query("SELECT number FROM numbers(5)"));
\`\`\`

SQL runs via POST with an explicit UTF-8 byte limit. Results return wide numeric values
as strings, not floating point numbers. The client has a deadline and response-size
limit and exposes HTTP failures with their status code. No automatic retries,
pagination, INSERT, or DDL.

The server has **no TLS or authentication**. Loopback is the safe default.
Only use private peers through a separately secured overlay with ACLs; never
expose the RustHouse HTTP port to the public Internet.

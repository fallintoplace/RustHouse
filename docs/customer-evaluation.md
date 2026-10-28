# RustHouse first-customer evaluation guide

This is an **experiment design**, not a promise of production support.
RustHouse currently excels as a small local Rust SQL/immutable-part tool.
Its current server and distributed capabilities remain experimental.

## Who should try it now

1. **Local analytical-tool developers** who need reproducible, bounded SQL
   against small local data or generated sources.
2. **ClickHouse part investigators** who possess a safely frozen snapshot and
   want independent metadata or read-only checks.
3. **Rust database contributors** who want to extend or measure the SQL,
   storage and CLI implementation.

## Who should not migrate yet

- Customers requiring high availability, multi-user RBAC, public HTTP access,
  managed multi-tenancy or cross-statement ACID transactions.
- Teams needing ClickHouse wire-protocol compatibility or full SQL coverage.
- Workloads that need live directory reads from another writer.
- Buyers expecting proven power-loss durability or unattended upgrades.

## One-hour evaluation, using today's supported CLI

\`\`\`sh
cargo build --release --locked -p rusthouse-cli
target/release/rusthouse sql "SELECT number FROM numbers(5) ORDER BY number DESC"
db="$(mktemp -d)"
target/release/rusthouse sql "CREATE TABLE events (id UInt64, region String)" --database "$db"
target/release/rusthouse sql "INSERT INTO events VALUES (1, 'EU'), (2, 'US')" --database "$db"
target/release/rusthouse sql "SELECT region, count(*) FROM events GROUP BY region ORDER BY region" --database "$db"
target/release/rusthouse doctor --database "$db"
\`\`\`

The test uses a disposable local database. Do not copy a live external part.
Local scripts are not cross-statement transactions.

## Pilot scorecard (proposed, measured 0-100)

- **Correctness 35 points:** exact expected rows and aggregates, errors
  correctly rejected, no silent truncation. Critical gate: no mismatches.
- **Time to value 20 points:** first successful local query and dataset import,
  with minutes and user interventions recorded.
- **Performance 20 points:** reference-verified median and p90 latency
  on fixed data, compiler, hardware and cache state. Never compare unlike runs.
- **Operability 15 points:** independent restart, offline doctor, successful
  cold restore, and documented recovery behavior.
- **Integration 10 points:** unambiguous HTTP/CLI contracts and reproducible
  client calls without out-of-band knowledge.

A score of **80+** is a *pilot candidate only*, not a declaration of
production readiness. Any correctness failure, missing required security
control or unverified data-loss mode is a no-go regardless of total score.

## Commercial learning goals

Interview five pilot teams and record: job-to-be-done, current alternative,
time lost per month, willingness to pay, blocker preventing adoption,
and whether a successful second trial was requested. Count **repeat weekly
usage** and **successful restores**, not only stars, PR counts or launch views.

Avoid claiming a valuation, revenue, correctness, or performance result from
this document. Convert repeated customer blockers into independently testable
engineering work before extending the surface area.

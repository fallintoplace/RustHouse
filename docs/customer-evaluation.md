# RustHouse first-customer evaluation guide

Use this guide to try RustHouse with local SQL and frozen parts. It helps you
measure the experience and check whether RustHouse fits your needs. The server
and distributed features are experimental.

## Who should try it now

1. **Local tool developers** who want to query small datasets or generated data.
2. **ClickHouse users** who want to inspect a frozen snapshot.
3. **Rust contributors** who want to improve or measure SQL, storage, or the CLI.

## Plan your evaluation

The current local database supports single-writer workloads and a SQL subset.
The HTTP server is read-only and has no built-in authentication. High
availability, multi-user RBAC, managed multi-tenancy, cross-statement
transactions, and ClickHouse wire compatibility are outside the current scope.
Use frozen snapshots for external parts. Power-loss durability and unattended
upgrades remain areas for further validation.

## Try RustHouse in one hour

```sh
cargo build --release --locked -p rusthouse-cli
target/release/rusthouse sql "SELECT number FROM numbers(5) ORDER BY number DESC"
db="$(mktemp -d)"
target/release/rusthouse sql "CREATE TABLE events (id UInt64, region String)" --database "$db"
target/release/rusthouse sql "INSERT INTO events VALUES (1, 'EU'), (2, 'US')" --database "$db"
target/release/rusthouse sql "SELECT region, count(*) FROM events GROUP BY region ORDER BY region" --database "$db"
target/release/rusthouse doctor --database "$db"
```

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

A score of **80 or more** means the project may be ready for a pilot. It does
not mean it is ready for production. A correctness failure, a missing required
security control, or an unverified data-loss mode blocks the pilot.

## Commercial learning goals

Interview five pilot teams. Record what they need, what they use today, how
much time they lose, and what blocks adoption. Ask whether they would pay and
try RustHouse again. Track **weekly repeat use** and **successful restores**.

This guide does not establish a valuation, revenue, correctness, or performance
result. Turn repeated blockers into work that can be tested independently.

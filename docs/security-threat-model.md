# RustHouse security threat model (experimental)

**Status: experimental and not production-certified.** This guide explains
the current security boundaries. RustHouse is not designed for hostile users.

## Assets and trust boundaries

- Database parts, schemas, and old snapshots can contain sensitive data. Use
  filesystem permissions to control access.
- The CLI accepts SQL and filenames from the local user. Do not build an
  untrusted service that inserts user input directly into SQL.
- The HTTP server can read SQL data and expose table and schema names.
  Read-only access is **not** the same as authorization or safe multi-tenancy.
- Coordinator and peer traffic uses plaintext HTTP by default. Opt-in HTTP bearer authentication is available, but peer clients do not yet forward credentials.
  A private RFC1918 or Tailscale address does not provide authentication.

## Threats ranked by deployment impact

1. **Public unauthenticated access: critical.** Anybody who can reach the
   HTTP port can send SELECT requests and potentially read local data.
   Mitigation today: loopback binding by default, no public exposure,
   restrictive firewall and encrypted private overlay ACLs if peers are used.
2. **Resource exhaustion: high.** Read-only SQL can still consume CPU, memory,
   I/O, connections and disk cache. Current byte/row/request bounds and
   connection caps limit some vectors, not total workload or per-user quotas.
   Do not accept arbitrary queries from untrusted clients.
3. **Live-part inconsistencies: high.** Reading an actively changing
   external MergeTree directory can produce inconsistent results.
   Use frozen snapshots only, as specified by the core documentation.
4. **Interrupted writes and recovery: high.** Process-exit testing does not
   establish power-loss durability; cross-statement scripts are not
   transactions. Keep tested, offline backups and verify restored copies.
5. **Symlinks and hostile filesystem contents: high.** Database readers must
   validate paths and metadata before trusting them. Treat files received
   from untrusted hosts as untrusted input and run in an isolated environment.
6. **Distributed integrity and freshness: high.** The peer-health endpoint
   checks reachability only. It does not validate freshness, permissions,
   replication, or atomic multi-shard snapshots.

## Controls you can use now

- Keep server bound to 127.0.0.1, or use a trusted encrypted overlay with
  strict peer-to-peer ACLs.
- Run as an unprivileged operating-system user and isolate databases by
  filesystem permissions. Never share a writable directory between
  uncoordinated writers.
- Stop ALL readers/writers before offline backup or doctor. Do not delete the
  persistent lock inode.
- Enforce request and query budgets outside RustHouse if exposing it behind
  another application. Do not treat a reverse proxy alone as row-level
  authorization.

## Work needed before a public or multi-tenant service

- Authenticated and authorized SQL/data access, with per-user and per-table
  permissions and auditable decisions.
- TLS transport with authenticated peers (or a documented cryptographic
  overlay boundary that is independently tested).
- Query cancellation, resource groups, global per-user limits, and abuse tests.
- Restore and power-loss fault testing on documented supported filesystems.
- Access audit events, operational metrics and incident-response procedures.
- Defined upgrade, downgrade, compatibility and secret-management policies.
- Tests against malicious part data, HTTP framing and distributed protocol
  mutations, plus external security review.

**No-go:** do not advertise compliance, enterprise RBAC, private cloud,
production HA, or safe public HTTP endpoints until the relevant controls are
implemented and independently verified.

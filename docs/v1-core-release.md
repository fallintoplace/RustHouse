# RustHouse v1 Core release contract (proposal)

This document defines a **proposed** scope and acceptance checklist for the first
stable RustHouse core release. It does not declare the current repository production-ready.
All release gates below must be verified against the final release candidate.

## Supported core scope

- Local, single-writer SQL databases using the explicitly documented statement
  and column-type subsets.
- Read-only queries against frozen, offline compatible wide MergeTree parts.
- CLI inspection, verification, bounded queries, and supported output formats.
- The local database diagnostic command (`rusthouse doctor`).

The SQL grammar is a documented subset, **not** ClickHouse compatibility.
Unsupported input must fail with a nonzero exit status instead of silently
returning truncated or incorrect results. Existing statement, part-count,
join-size, and import limits remain part of the contract until deliberately
changed and tested.

## Experimental scope (not part of the stable v1 contract)

- HTTP server and all peer/coordinator/distributed modes.
- Concurrent production writes, replication, automatic failover, and recovery
  across machines.
- Reading directories while another database engine modifies their parts.
- Cross-statement transactions and automatic data migrations.

The server's peer protocol has no built-in TLS or authentication. Do not expose
it to an untrusted network. A distributed result is not an atomic snapshot
across shards.

## Persistence guarantees

A local statement publishes immutable part files and, where applicable,
an active-generation pointer. A script of multiple statements is **not** one
transaction. Tests that terminate a process at publication boundaries are
useful, but they do not prove survival of sudden machine power loss.

Until a power-loss test program exists, do **not** advertise a general
power-loss durability or zero-data-loss guarantee. The initial stable release
must document the filesystem and operating systems used in storage testing.

### Offline backup and restore

1. Stop the CLI writers and HTTP readers before copying a database directory.
   Do not use a live, unsynchronized directory copy as a backup.
2. Preserve the *entire* database directory, including `*.schema`,
   `*.parts`, active snapshot pointers, retained generations, and files with
   leading dots. Do not manually delete the persistent lock inode.
3. Copy into a fresh destination. Do not restore over an active database.
4. Run `rusthouse doctor --database <restored-directory> --full-hashes`
   while all processes are stopped.
5. Reopen the restored database and compare row counts and representative
   values with the source. Test that a subsequent supported write succeeds.

The doctor command validates active parts. It does not certify inactive
generations, every conceivable corruption type, or power-loss safety.

## Release gates

- [ ] All mandatory checks succeed on the exact candidate commit: format,
      lint, Linux and macOS tests, minimum Rust version, and documented
      Windows checks.
- [ ] Every documented supported SQL case in the pinned correctness suite
      matches its expected result; unsupported cases are reported separately.
- [ ] Snapshot publication, process restart, malformed-part, and bounded-input
      cases pass. Restore-from-cold-backup tests pass.
- [ ] Version compatibility between the release candidate and supported
      existing on-disk layouts is explicit and tested.
- [ ] Reference-verified benchmarks report correctness independently from
      timings. Publish hardware, compiler, dataset and warm/cold-cache scope.
- [ ] Clean-install binaries, SHA-256 checksums, and release notes are verified
      for each *fully supported* platform.
- [ ] Known limitations, upgrade instructions, and rollback guidance are
      accurate and refer to the released commit.

## Versioning policy to adopt before tagging v1.0

For the stable release, changes to CLI behavior, supported SQL semantics,
persistent data format, and user-visible error guarantees must be reviewed
for compatibility. Do not bump the version to v1.0 without determining how
older local data directories are detected, upgraded if necessary, and
recovered if an upgrade is interrupted.

This is an acceptance document, not an assertion that these promises already
hold. The primary project README remains the source for current features.

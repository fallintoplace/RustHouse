# Local database backup and recovery runbook

RustHouse is experimental. This guide explains how to back up and restore a
local database while it is stopped. It does not guarantee high availability,
online backup safety, or durability after a crash.

## Before you start

- Use a trusted local filesystem. Do not expose the HTTP server to the public
  internet; it has no built-in authentication.
- Stop all RustHouse readers and writers, including the HTTP server. Stop any
  other process that copies or changes the database. Wait for them to exit.
- Check that there is enough free disk space to copy the entire database.
  Include hidden files and every `.schema` file. The `.parts` folder may also
  contain `.rusthouse-active`, `.rusthouse-versions`, and older generations.
  Do not copy only the visible `all_*` part folders.
- Check that the source database is readable and consistent:

```sh
rusthouse doctor --database /srv/rusthouse/live --full-hashes
```

## Create a cold backup

Use a new, empty destination and preserve paths, metadata, and hidden files:

```sh
mkdir -p /srv/rusthouse/backup-20261030
cp -a /srv/rusthouse/live/. /srv/rusthouse/backup-20261030/
rusthouse doctor --database /srv/rusthouse/backup-20261030 --full-hashes
```

These commands need a Unix-like system with `cp -a`. Keep the original folder
unchanged until the check finishes. A successful copy alone does not prove the
backup is durable. Use the right sync and storage checks for your filesystem,
and keep a copy on another machine.

Do not back up while writes, merges, compactions or schema changes are active.
A copy of live files can contain a mix of incompatible generations.

## Test a restore before relying on the backup

Restore to a **fresh** directory, not over a running or populated database:

```sh
mkdir -p /srv/rusthouse/restore-check
cp -a /srv/rusthouse/backup-20261030/. /srv/rusthouse/restore-check/
rusthouse doctor --database /srv/rusthouse/restore-check --full-hashes
rusthouse sql 'SHOW TABLES' --database /srv/rusthouse/restore-check
```

For each table you need, check the row count and run a sample query. Compare
the results with a manifest saved before the backup. A full hash check can find
corruption, but it cannot tell whether a valid snapshot is older than expected.

Send writes to the restored folder only after the checks pass and you have a
cutover plan. Do not run concurrent writers on the same database path.

## On failure

- **Missing schema or pointer:** preserve the entire directory for diagnosis.
  Do not guess or rewrite `.rusthouse-active`.
- **Corrupt part or checksum mismatch:** fail closed; use an independently
  verified copy. Do not delete data to make the integrity scan pass.
- **Incomplete backup:** do not attempt a partial merge into a live directory.
  Start a new cold copy after stopping writers.
- **Unknown crash point / ambiguous success:** use the read-only doctor and
  SQL validation before resuming writes.

Capture the CLI error, filesystem type, OS version, database directory
inventory and failing command. Do not include sensitive table contents in
public issue reports.

## What this runbook does not establish

A successful restore drill does not establish acceptable RPO/RTO, power-loss
durability, distributed consensus, replica consistency, TLS, authorization,
online backup safety or automatic failover. Those require separate tests,
operational controls and reviewed release gates. See
[v1-core-release.md](v1-core-release.md),
[security-threat-model.md](security-threat-model.md), and
[customer-evaluation.md](customer-evaluation.md).

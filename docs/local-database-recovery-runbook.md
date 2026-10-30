# Local database backup and recovery runbook

RustHouse is experimental. This document describes a **cold, single-writer,
local-filesystem** recovery workflow. It is not an HA, online-backup, or
crash-durability guarantee.

## Preconditions

- Run on a trusted local filesystem. Do not expose the unauthenticated HTTP
  listener to the public internet.
- Stop all RustHouse writers and any process copying or rewriting the database.
  Wait until writes have exited. The `.parts` directory may contain
  `.rusthouse-active`, `.rusthouse-versions`, and immutable generations.
- Check free disk capacity before copying the entire database. Include hidden
  entries and every `.schema` file. Do not copy just the currently visible
  `all_*` part directories.
- Confirm that the source database is readable and consistent:

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

The commands above assume a Unix-like system with `cp -a`. Keep the
original directory unchanged until verification completes. A completed file
copy does not by itself certify durable persistence on the backup device:
use filesystem-appropriate sync, storage checks, and off-machine retention.

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

For each required table, run a known row count and a representative query.
Compare results with an independently captured manifest from before the
backup. A full hash check detects corruption; it cannot tell whether an
otherwise valid but older snapshot was accidentally selected.

Only redirect writes to the restored directory after validation and a
planned cutover. Never run concurrent writers against one database path.

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

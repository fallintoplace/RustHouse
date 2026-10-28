# Verify a cold RustHouse backup

**Stop all writers and HTTP readers before invoking this utility.** It does
not acquire database locks or make online, cross-statement snapshots.

```sh
cargo build -p rusthouse-cli --locked
python3 tools/offline_backup.py ./mydb ./mydb-backup --offline-confirmed
```

The destination must be new and outside the source. The script rejects
symlinks, copies the entire directory including dotfiles into a new
`.partial` staging directory, runs `doctor --full-hashes`, and renames
the staged copy on success. A failed check leaves staging intact for manual
inspection and never deletes the source. A successful check does **not**
prove power-loss durability, inactive-generation integrity, or absence of
an undetected concurrent writer. Test a restore before relying on backups.

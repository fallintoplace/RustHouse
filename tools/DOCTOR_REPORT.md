# Offline doctor as machine-readable JSON

Stop RustHouse writers and HTTP readers first. Then:

```sh
python3 tools/doctor_report.py --database ./mydb --offline-confirmed > doctor.json
```

The wrapper records whether the existing `rusthouse doctor` command succeeded
and preserves bounded textual output. It does not parse internal diagnostic
messages, repair data, or claim to verify inactive generations or power-loss
durability. `--quick` skips the more expensive stored-byte full-hash check.
The command does **not** acquire the database write lock; operator quiescence
is mandatory. Keep reports private if file paths are sensitive.

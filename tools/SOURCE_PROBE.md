# Verify the configured RustHouse source

```sh
python3 tools/source_probe.py --source generated
python3 tools/source_probe.py --source database --table events
```

The probe validates the read-only capability marker and the source mode
reported by the server. A database-mode probe can also verify schema discovery.
It refuses remote HTTP origins because the built-in server has no TLS or
authentication. It does not guarantee row freshness or storage durability.

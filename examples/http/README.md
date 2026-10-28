# Read-only HTTP acceptance check

Start `rusthouse-server --port 8123` in another terminal, then run:

```sh
python3 examples/http/smoke.py
cd examples/http && python3 -m unittest discover -v
```

Verifies exact `numbers(3)` rows, the plain-text ping, read-only capabilities
and EXPLAIN. It never changes a database. It defaults to loopback because the
experimental HTTP server has no TLS or authentication. Never expose it publicly.

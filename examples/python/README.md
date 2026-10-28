# RustHouse Python HTTP client (experimental)

This dependency-free Python 3.9+ example uses the read-only RustHouse HTTP
interface. It is not a production SDK or a ClickHouse-compatible driver.

Start the server on loopback (use `--database` for named local tables):

```sh
cargo run -p rusthouse-server -- --port 8123
cd examples/python
python3 -m unittest discover -v
```

```python
from rusthouse_client import RustHouseClient

client = RustHouseClient()
print(client.capabilities())
print(client.query("SELECT number FROM numbers(3)"))
print(client.explain("SELECT count(*) FROM numbers(10)"))
```

- The client issues raw SELECT SQL via POST; it does not offer INSERT or DDL.
- JSON numeric values are strings to preserve wide integer precision.
- HTTP 4xx/5xx raises `RustHouseHTTPError` with `status` and `body`.
- Non-loopback endpoints require explicit `allow_private_network=True`, and
  must use a trusted encrypted overlay with ACLs. The server itself has no TLS
  or authentication. Never expose it to the public Internet.
- Responses, SQL byte size, and socket wait time are bounded; server-side
  limits still apply, and there is no automatic retries or pagination.

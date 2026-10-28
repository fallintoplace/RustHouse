import test from "node:test";
import assert from "node:assert/strict";
import { RustHouseClient, RustHouseError } from "./rusthouse-client.mjs";

function reply(body, status = 200) {
  return new Response(body, { status, headers: { "Content-Type": "application/json" } });
}
test("blocks untrusted network by default", () => {
  assert.throws(() => new RustHouseClient("http://public.example:8123"));
  assert.throws(() => new RustHouseClient("http://127.0.0.1:8123/?token=x"));
  assert.doesNotThrow(() => new RustHouseClient("http://10.2.0.5:8123", { allowPrivateNetwork: true }));
});
test("raw POST preserves SQL and wide integers", async () => {
  let request;
  const client = new RustHouseClient(undefined, {
    fetchImpl: async (url, opts) => { request = { url, opts }; return reply('[{"n":"18446744073709551615"}]'); }
  });
  assert.equal((await client.query("SELECT 1"))[0].n, "18446744073709551615");
  assert.equal(request.url, "http://127.0.0.1:8123/?format=json");
  assert.equal(request.opts.method, "POST");
  assert.equal(new TextDecoder().decode(request.opts.body), "SELECT 1");
});
test("schema safely encodes the path parameter", async () => {
  let path;
  const client = new RustHouseClient(undefined, {
    fetchImpl: async url => { path = url; return reply('{"columns":[]}'); }
  });
  await client.schema("a&b");
  assert.ok(path.endsWith("?table=a%26b"));
});
test("surfaces SQL errors with HTTP status", async () => {
  const client = new RustHouseClient(undefined, { fetchImpl: async () => reply("bad SQL", 400) });
  await assert.rejects(client.query("SELECT bad"), e => e instanceof RustHouseError && e.status === 400);
});
test("rejects oversized SQL", async () => {
  const client = new RustHouseClient();
  await assert.rejects(client.query("é".repeat(3000)), RangeError);
});

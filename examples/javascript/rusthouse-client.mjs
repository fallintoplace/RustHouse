// Dependency-free Node.js 18+ client for RustHouse's experimental read-only HTTP API.
// No automatic retries. Numeric JSON values remain strings on purpose.
export class RustHouseError extends Error {
  constructor(message, status = null) {
    super(message);
    this.name = "RustHouseError";
    this.status = status;
  }
}

export class RustHouseClient {
  constructor(origin = "http://127.0.0.1:8123", {
    timeoutMs = 5000, allowPrivateNetwork = false, fetchImpl = globalThis.fetch
  } = {}) {
    const url = new URL(origin);
    if (url.protocol !== "http:" || url.username || url.password ||
        (url.pathname !== "/" && url.pathname !== "") || url.search || url.hash ||
        !Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 120000) {
      throw new TypeError("expected HTTP origin and timeoutMs in 1..120000");
    }
    if (!["localhost", "127.0.0.1"].includes(url.hostname) && !allowPrivateNetwork) {
      throw new TypeError("non-loopback requires explicit allowPrivateNetwork");
    }
    if (typeof fetchImpl !== "function") throw new TypeError("fetch is required");
    this.origin = url.origin;
    this.timeoutMs = timeoutMs;
    this.fetch = fetchImpl;
  }

  async request(path, sql = null) {
    const bytes = sql === null ? null : new TextEncoder().encode(sql);
    if (bytes !== null && (bytes.length === 0 || bytes.length > 4096)) {
      throw new RangeError("SQL must contain 1..4096 UTF-8 bytes");
    }
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      const response = await this.fetch(this.origin + path, {
        method: sql === null ? "GET" : "POST",
        ...(bytes === null ? {} : { body: bytes, headers: { "Content-Type": "text/plain" } }),
        signal: controller.signal
      });
      // Enforce the size limit before materializing an untrusted response.
      const length = Number(response.headers?.get("content-length"));
      if (Number.isFinite(length) && length > 16 * 1024 * 1024) {
        throw new RustHouseError("response exceeds 16 MiB");
      }
      const reader = response.body?.getReader();
      if (!reader) throw new RustHouseError("response stream missing");
      let total = 0;
      const chunks = [];
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        total += value.byteLength;
        if (total > 16 * 1024 * 1024) {
          await reader.cancel();
          throw new RustHouseError("response exceeds 16 MiB");
        }
        chunks.push(value);
      }
      const output = new Uint8Array(total);
      let offset = 0;
      for (const chunk of chunks) { output.set(chunk, offset); offset += chunk.length; }
      const text = new TextDecoder("utf-8", { fatal: true }).decode(output);
      if (!response.ok) throw new RustHouseError(
        "RustHouse HTTP " + response.status + ": " + text.slice(0, 200).trim(),
        response.status
      );
      return text;
    } catch (error) {
      if (controller.signal.aborted) throw new RustHouseError("RustHouse request timed out");
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }

  async json(path, sql = null) {
    try {
      return JSON.parse(await this.request(path, sql));
    } catch (error) {
      if (error instanceof SyntaxError) throw new RustHouseError("invalid JSON from RustHouse");
      throw error;
    }
  }

  capabilities() { return this.json("/capabilities"); }
  tables() { return this.json("/tables"); }
  schema(table = null) {
    if (table === null) return this.json("/schema");
    if (typeof table !== "string" || !table || new TextEncoder().encode(table).length > 128)
      throw new TypeError("invalid table name");
    return this.json("/schema?table=" + encodeURIComponent(table));
  }
  async query(sql) {
    const rows = await this.json("/?format=json", sql);
    if (!Array.isArray(rows)) throw new RustHouseError("expected a JSON array");
    return rows;
  }
  explain(sql) { return this.request("/explain", sql); }
}

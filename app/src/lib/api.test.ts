/**
 * The client against a stub server, started by the test.
 *
 * A stub rather than a mocked `fetch`, for the reason `ScriptedSource` is a real
 * `LogSource` on the Rust side: patching the thing under test's dependency tests
 * the patch. This exercises the actual request, the actual status handling and
 * the actual JSON parse.
 *
 * What it deliberately does not verify is that these shapes match `api.rs` —
 * nothing here can, since the two are hand-written on either side of a network
 * boundary. That agreement is checked by running the real binary; see
 * `indexer/README.md`.
 */
import { strict as assert } from "node:assert";
import { createServer, type Server } from "node:http";
import { after, before, test } from "node:test";

import { createClient } from "./api.ts";

/** Paths the stub was asked for, so the client's URL building is observable. */
const requested: string[] = [];
let server: Server;
let base: string;

before(async () => {
  server = createServer((req, res) => {
    requested.push(req.url ?? "");

    if (req.url?.startsWith("/health")) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ finalized_slot: 42, pending_transactions: 3 }));
      return;
    }
    if (req.url?.startsWith("/pools/known")) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          meta: { finality: "finalized", slot: 42, pending_transactions: 0 },
          // Past 2^53, which is why the wire format uses strings.
          data: { total_staked: "9007199254740993", apr_bps: null },
        }),
      );
      return;
    }
    if (req.url?.startsWith("/pools/broken")) {
      res.writeHead(500);
      res.end();
      return;
    }
    res.writeHead(404);
    res.end();
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (typeof address === "string" || address === null) throw new Error("no port");
  base = `http://127.0.0.1:${address.port}`;
});

after(() => server.close());

test("a successful response is parsed, large amounts intact", async () => {
  const result = await createClient(base).pool("known", "finalized");
  assert.ok(result.ok);
  assert.equal(result.value.meta.slot, 42);
  assert.equal(result.value.data.total_staked, "9007199254740993");
  assert.equal(result.value.data.apr_bps, null);
});

test("404 is not-found, distinct from an error", async () => {
  const result = await createClient(base).pool("unknown", "finalized");
  assert.ok(!result.ok);
  assert.equal(result.failure.kind, "not-found");
});

test("a server error keeps its status", async () => {
  const result = await createClient(base).pool("broken", "finalized");
  assert.ok(!result.ok);
  assert.equal(result.failure.kind, "error");
  if (result.failure.kind === "error") assert.equal(result.failure.status, 500);
});

test("an unreachable API is its own outcome, not a thrown error", async () => {
  // Port 1 is reserved and nothing listens on it.
  const result = await createClient("http://127.0.0.1:1").health();
  assert.ok(!result.ok);
  assert.equal(result.failure.kind, "unreachable");
});

test("finality is only sent when it is not the default", async () => {
  const client = createClient(base);
  requested.length = 0;

  await client.pool("known", "finalized");
  await client.pool("known", "head");
  await client.stakers("known", "finalized", 5);
  await client.stakers("known", "head", 5);

  assert.equal(requested[0], "/pools/known", "the default should not be sent");
  assert.equal(requested[1], "/pools/known?finality=head");
  // The stakers route already has a query string, so the separator differs —
  // the case a naive `?finality=` would break.
  assert.equal(requested[2], "/pools/known/stakers?limit=5");
  assert.equal(requested[3], "/pools/known/stakers?limit=5&finality=head");
});

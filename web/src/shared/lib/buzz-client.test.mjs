import assert from "node:assert/strict";
import { test } from "node:test";

import { KINDS } from "@/shared/constants/kinds.ts";
import { BuzzClient } from "./buzz-client.ts";
import {
  EphemeralSigner,
  LocalKeySigner,
  Nip07Signer,
  Nip07UnavailableError,
} from "./nostr-signer.ts";

/**
 * Minimal mock WebSocket (no real network). Controllers call `peerSend` to
 * inject server frames; client writes land in `sent`.
 */
class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;

  url;
  readyState = MockWebSocket.CONNECTING;
  sent = [];
  #listeners = new Map();

  constructor(url) {
    this.url = url;
    // Open asynchronously, like a real socket.
    queueMicrotask(() => {
      this.readyState = MockWebSocket.OPEN;
      this.#emit("open", {});
    });
  }

  addEventListener(type, listener) {
    if (!this.#listeners.has(type)) this.#listeners.set(type, new Set());
    this.#listeners.get(type).add(listener);
  }

  removeEventListener(type, listener) {
    this.#listeners.get(type)?.delete(listener);
  }

  send(data) {
    if (this.readyState !== MockWebSocket.OPEN) {
      throw new Error("InvalidStateError");
    }
    this.sent.push(data);
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.#emit("close", {});
  }

  /** Inject a server → client message. */
  peerSend(frame) {
    this.#emit("message", { data: JSON.stringify(frame) });
  }

  #emit(type, ev) {
    for (const listener of this.#listeners.get(type) ?? []) {
      listener(ev);
    }
  }
}

// buzz-client.ts reads the global WebSocket for its OPEN/CLOSED constants.
globalThis.WebSocket = MockWebSocket;

function findSent(ws, predicate) {
  for (const raw of ws.sent) {
    const frame = JSON.parse(raw);
    if (predicate(frame)) return frame;
  }
  return undefined;
}

function sentFrames(ws, type) {
  return ws.sent
    .map((raw) => JSON.parse(raw))
    .filter((frame) => frame[0] === type);
}

function fakeEvent(overrides) {
  return {
    id: "e".repeat(64),
    pubkey: "p".repeat(64),
    created_at: 100,
    kind: KINDS.STREAM_MESSAGE,
    tags: [["h", "lobby"]],
    content: "hello",
    sig: "s".repeat(128),
    ...overrides,
  };
}

async function flush() {
  // Drain microtasks used by auth grace / async handlers.
  await Promise.resolve();
  await Promise.resolve();
}

let sockets = [];
function factory(url) {
  const ws = new MockWebSocket(url);
  sockets.push(ws);
  return ws;
}

test("EphemeralSigner signs events with a stable page-lifetime pubkey", async () => {
  const signer = new EphemeralSigner();
  const pk = await signer.getPublicKey();
  assert.match(pk, /^[0-9a-f]{64}$/);

  const signed = await signer.signEvent({
    kind: KINDS.STREAM_MESSAGE,
    created_at: 10,
    tags: [["h", "c"]],
    content: "x",
  });
  assert.equal(signed.pubkey, pk);
  assert.match(signed.id, /^[0-9a-f]{64}$/);
  assert.match(signed.sig, /^[0-9a-f]{128}$/);
  assert.equal(signer.type, "ephemeral");
});

test("LocalKeySigner signs events with a stable pubkey for an explicit key", async () => {
  const signer = new LocalKeySigner(new Uint8Array(32).fill(7));
  const pk = await signer.getPublicKey();
  const again = await signer.getPublicKey();
  assert.equal(pk, again);
  const signed = await signer.signEvent({
    kind: KINDS.STREAM_MESSAGE,
    created_at: 10,
    tags: [],
    content: "x",
  });
  assert.equal(signed.pubkey, pk);
  assert.equal(signer.type, "local");
});

test("Nip07Signer throws when no browser extension is present", async () => {
  const signer = new Nip07Signer();
  assert.equal(signer.type, "nip07");
  await assert.rejects(
    () => signer.getPublicKey(),
    (error) => error instanceof Nip07UnavailableError,
  );
});

test("BuzzClient", async (t) => {
  t.afterEach(() => {
    sockets = [];
  });

  await t.test(
    "handles AUTH challenge/response then becomes connected",
    async () => {
      const signer = new EphemeralSigner();
      const client = new BuzzClient({
        relayUrl: "ws://relay.test",
        signer,
        authGraceMs: 50,
        webSocketFactory: factory,
      });

      const connectP = client.connect();
      await flush();
      assert.equal(sockets.length, 1);
      const ws = sockets[0];

      ws.peerSend(["AUTH", "challenge-xyz"]);
      await flush();
      await flush();

      const authFrame = findSent(
        ws,
        (f) => f[0] === "AUTH" && typeof f[1] === "object",
      );
      assert.ok(authFrame, "client must respond to the AUTH challenge");
      const signed = authFrame[1];
      assert.equal(signed.kind, KINDS.AUTH);
      assert.deepEqual(
        signed.tags.find((t) => t[0] === "challenge"),
        ["challenge", "challenge-xyz"],
      );

      ws.peerSend(["OK", signed.id, true, ""]);
      await connectP;
      assert.equal(client.getConnectionState(), "connected");

      client.disconnect();
    },
  );

  await t.test(
    "subscribes with explicit kinds and #h tag, collects EVENT then EOSE",
    async () => {
      const signer = new EphemeralSigner();
      const client = new BuzzClient({
        relayUrl: "ws://relay.test",
        signer,
        authGraceMs: 5,
        webSocketFactory: factory,
      });

      const received = [];
      let eoseHit = false;

      const unsub = client.subscribeTimeline(
        "lobby",
        (ev) => {
          received.push(ev);
        },
        { limit: 10, onEose: () => (eoseHit = true) },
      );

      await new Promise((r) => setTimeout(r, 30));
      assert.equal(client.getConnectionState(), "connected");
      const ws = sockets[0];

      const req = findSent(ws, (f) => f[0] === "REQ");
      assert.ok(req);
      const subId = req[1];
      const filter = req[2];
      assert.deepEqual(filter.kinds, [
        KINDS.STREAM_MESSAGE,
        KINDS.STREAM_MESSAGE_V2,
        KINDS.REACTION,
      ]);
      assert.deepEqual(filter["#h"], ["lobby"]);
      assert.equal(filter.limit, 10);

      ws.peerSend(["EVENT", subId, fakeEvent({ id: "e1".padEnd(64, "0") })]);
      ws.peerSend(["EOSE", subId]);
      await flush();
      assert.equal(received.length, 1);
      assert.equal(received[0].content, "hello");
      assert.equal(eoseHit, true);
      assert.equal(client.getTimeline("lobby").length, 1);

      ws.peerSend([
        "EVENT",
        subId,
        fakeEvent({
          id: "e2".padEnd(64, "0"),
          content: "live",
          created_at: 101,
        }),
      ]);
      await flush();
      assert.equal(received.length, 2);
      assert.equal(received[1].content, "live");

      unsub();
      client.disconnect();
    },
  );

  await t.test("publishes kind-9 and resolves on OK", async () => {
    const signer = new EphemeralSigner();
    const client = new BuzzClient({
      relayUrl: "ws://relay.test",
      signer,
      authGraceMs: 5,
      webSocketFactory: factory,
    });

    await client.connect();
    await new Promise((r) => setTimeout(r, 20));
    assert.equal(client.getConnectionState(), "connected");

    const ws = sockets[0];
    const sendP = client.sendMessage("lobby", "ping", "parent-id");
    await flush();
    await flush();

    const eventFrame = findSent(ws, (f) => f[0] === "EVENT");
    assert.ok(eventFrame);
    const published = eventFrame[1];
    assert.equal(published.kind, KINDS.STREAM_MESSAGE);
    assert.equal(published.content, "ping");
    assert.deepEqual(
      published.tags.find((t) => t[0] === "h"),
      ["h", "lobby"],
    );
    assert.deepEqual(
      published.tags.find((t) => t[0] === "e"),
      ["e", "parent-id", "", "reply"],
    );

    ws.peerSend(["OK", published.id, true, ""]);
    const ack = await sendP;
    assert.equal(ack.ok, true);
    assert.equal(ack.id, published.id);

    client.disconnect();
  });

  await t.test("rejects on AUTH failure", async () => {
    const signer = new EphemeralSigner();
    const client = new BuzzClient({
      relayUrl: "ws://relay.test",
      signer,
      authGraceMs: 200,
      minReconnectDelayMs: 5,
      maxReconnectDelayMs: 5,
      webSocketFactory: factory,
    });

    const connectP = client.connect();
    await flush();
    const ws = sockets[0];
    ws.peerSend(["AUTH", "bad-chal"]);
    await flush();
    await flush();

    const authFrame = findSent(
      ws,
      (f) => f[0] === "AUTH" && typeof f[1] === "object",
    );
    const signed = authFrame[1];
    ws.peerSend(["OK", signed.id, false, "auth-required: invalid"]);

    await assert.rejects(connectP, /invalid|authentication/i);
    assert.equal(client.getConnectionState(), "error");

    client.disconnect();
  });

  await t.test(
    "reconnects with backoff after an unexpected close and re-sends the timeline subscription",
    async () => {
      const signer = new EphemeralSigner();
      const client = new BuzzClient({
        relayUrl: "ws://relay.test",
        signer,
        authGraceMs: 5,
        minReconnectDelayMs: 5,
        maxReconnectDelayMs: 20,
        webSocketFactory: factory,
      });

      const received = [];
      client.subscribeTimeline("lobby", (ev) => received.push(ev));
      await new Promise((r) => setTimeout(r, 30));
      assert.equal(client.getConnectionState(), "connected");
      assert.equal(sockets.length, 1);

      // Simulate an unexpected network drop — not client.disconnect().
      sockets[0].close();

      // The scheduled reconnect must open a second socket and re-authenticate.
      await new Promise((r) => setTimeout(r, 80));
      assert.equal(sockets.length, 2, "reconnect must open a new socket");
      assert.equal(client.getConnectionState(), "connected");
      const secondWs = sockets[1];

      const req = findSent(secondWs, (f) => f[0] === "REQ");
      assert.ok(req, "the timeline subscription must survive the reconnect");
      const subId = req[1];
      assert.deepEqual(req[2]["#h"], ["lobby"]);

      secondWs.peerSend([
        "EVENT",
        subId,
        fakeEvent({
          id: "e3".padEnd(64, "0"),
          content: "after-reconnect",
          created_at: 200,
        }),
      ]);
      await flush();
      assert.equal(received.at(-1).content, "after-reconnect");

      client.disconnect();
    },
  );

  await t.test(
    "handles a mid-session AUTH re-challenge without tearing down subscriptions",
    async () => {
      const signer = new EphemeralSigner();
      const client = new BuzzClient({
        relayUrl: "ws://relay.test",
        signer,
        authGraceMs: 5,
        webSocketFactory: factory,
      });

      const received = [];
      client.subscribeTimeline("lobby", (ev) => received.push(ev));
      await new Promise((r) => setTimeout(r, 30));
      assert.equal(client.getConnectionState(), "connected");
      const ws = sockets[0];
      assert.equal(sentFrames(ws, "REQ").length, 1);

      // Relay re-challenges on the SAME socket — it is never closed. (The
      // initial connect above completed via the auth-grace timeout, with no
      // challenge, so this is the only AUTH exchange in this test.)
      ws.peerSend(["AUTH", "second-challenge"]);
      await flush();
      assert.equal(client.getConnectionState(), "authenticating");

      const authFrames = ws.sent
        .map((raw) => JSON.parse(raw))
        .filter((f) => f[0] === "AUTH" && typeof f[1] === "object");
      assert.equal(authFrames.length, 1, "must respond to the re-challenge");
      const secondSigned = authFrames[0][1];
      assert.deepEqual(
        secondSigned.tags.find((t) => t[0] === "challenge"),
        ["challenge", "second-challenge"],
      );

      ws.peerSend(["OK", secondSigned.id, true, ""]);
      await flush();
      assert.equal(client.getConnectionState(), "connected");

      // The existing subscription must not have been torn down or resent.
      assert.equal(sentFrames(ws, "REQ").length, 1);

      const subId = sentFrames(ws, "REQ")[0][1];
      ws.peerSend([
        "EVENT",
        subId,
        fakeEvent({
          id: "e4".padEnd(64, "0"),
          content: "still-alive",
          created_at: 300,
        }),
      ]);
      await flush();
      assert.equal(received.at(-1).content, "still-alive");

      client.disconnect();
    },
  );
});

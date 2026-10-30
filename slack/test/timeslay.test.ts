import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  addToInbox,
  decodeNativeMessage,
  encodeNativeMessage,
} from "../src/timeslay.ts";

const directory = mkdtempSync(join(tmpdir(), "timeslay-slack-"));
process.on("exit", () => rmSync(directory, { recursive: true, force: true }));

/**
 * A tiny stand-in for `timeslay --webextension`: it reads one framed request,
 * records it to FAKE_HOST_OUT, and answers with FAKE_HOST_REPLY.
 */
const FAKE_HOST = `#!/usr/bin/env node
const fs = require("node:fs");
const { endianness } = require("node:os");

function readMessage(buffer) {
  if (buffer.length < 4) return null;
  const length =
    endianness() === "LE" ? buffer.readUInt32LE(0) : buffer.readUInt32BE(0);
  if (buffer.length < 4 + length) return null;
  return { value: JSON.parse(buffer.subarray(4, 4 + length).toString("utf8")), size: 4 + length };
}

function frame(value) {
  const body = Buffer.from(JSON.stringify(value), "utf8");
  const header = Buffer.allocUnsafe(4);
  if (endianness() === "LE") header.writeUInt32LE(body.length, 0);
  else header.writeUInt32BE(body.length, 0);
  return Buffer.concat([header, body]);
}

const chunks = [];
process.stdin.on("data", (chunk) => chunks.push(chunk));
process.stdin.on("end", () => {
  const buffer = Buffer.concat(chunks);
  const request = readMessage(buffer);
  if (request && process.env.FAKE_HOST_OUT) {
    fs.writeFileSync(process.env.FAKE_HOST_OUT, JSON.stringify(request.value));
  }
  if (process.env.FAKE_HOST_HANG === "1") {
    setTimeout(() => process.exit(0), 30000);
    return;
  }
  const reply = process.env.FAKE_HOST_RAW
    ? Buffer.from(process.env.FAKE_HOST_RAW)
    : process.env.FAKE_HOST_REPLY
      ? frame(JSON.parse(process.env.FAKE_HOST_REPLY))
      : Buffer.from("not a native message");
  process.stdout.write(reply);
  process.stdout.end();
});
`;

function hostPath(name: string): string {
  const path = join(directory, name);
  writeFileSync(path, FAKE_HOST, { mode: 0o755 });
  return path;
}

const SUCCESS = JSON.stringify({
  ok: true,
  task: { id: "task:1", title: "A task", link: "https://tasks.google.com/1" },
  via: "direct",
});

test("addToInbox sends a framed request and reads the framed reply", async () => {
  const bin = hostPath("ok.cjs");
  const out = join(directory, "request.json");
  process.env.FAKE_HOST_OUT = out;
  process.env.FAKE_HOST_REPLY = SUCCESS;

  const result = await addToInbox(
    {
      command: "add-to-inbox",
      url: "https://example.org/article",
      title: "An article",
      description: "Read this",
      source: "slack",
      due: "2026-11-02",
      deadline: "2026-11-02 17:00",
    },
    { bin },
  );

  assert.equal(result.ok, true);
  assert.equal(result.task?.title, "A task");
  assert.equal(result.via, "direct");

  const sent = JSON.parse(readFileSync(out, "utf8"));
  assert.equal(sent.command, "add-to-inbox");
  assert.equal(sent.url, "https://example.org/article");
  assert.equal(sent.source, "slack");
  assert.equal(sent.due, "2026-11-02");
  assert.equal(sent.deadline, "2026-11-02 17:00");
});

test("addToInbox reports a failure reply with its trace", async () => {
  const bin = hostPath("fail.cjs");
  process.env.FAKE_HOST_REPLY = JSON.stringify({
    ok: false,
    error: "Could not read the link",
    trace: "Steps:\nasking timeslay-window.sock",
  });

  const result = await addToInbox(
    { url: "https://example.org/", title: "x", description: "y" },
    { bin },
  );

  assert.equal(result.ok, false);
  assert.equal(result.error, "Could not read the link");
  assert.match(result.trace ?? "", /timeslay-window\.sock/);
});

test("addToInbox rejects a reply it cannot decode", async () => {
  delete process.env.FAKE_HOST_REPLY;
  const bin = hostPath("garbage.cjs");

  const result = await addToInbox(
    { url: "https://example.org/", title: "x", description: "y" },
    { bin },
  );

  assert.equal(result.ok, false);
  assert.match(result.error ?? "", /does not understand|Could not decode/);
});

test("addToInbox times out when the host never answers", async () => {
  delete process.env.FAKE_HOST_REPLY;
  process.env.FAKE_HOST_HANG = "1";
  const bin = hostPath("hang.cjs");

  const result = await addToInbox(
    { url: "https://example.org/", title: "x", description: "y" },
    { bin, timeoutMs: 300 },
  );

  assert.equal(result.ok, false);
  assert.match(result.error ?? "", /Timed out after 300ms/);
  delete process.env.FAKE_HOST_HANG;
});

test("addToInbox explains a missing binary", async () => {
  const result = await addToInbox(
    { url: "https://example.org/", title: "x", description: "y" },
    { bin: join(directory, "does-not-exist") },
  );

  assert.equal(result.ok, false);
  assert.match(result.error ?? "", /Could not run/);
});

test("native framing round-trips with the host", () => {
  const value = { command: "add-to-inbox", n: 1, ok: true };
  assert.deepEqual(decodeNativeMessage(encodeNativeMessage(value)), value);
  assert.equal(decodeNativeMessage(Buffer.from([0, 0])), null);
});

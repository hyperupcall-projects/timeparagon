import assert from "node:assert/strict";
import test from "node:test";

import type { WebClient } from "@slack/web-api";

import {
  contextFor,
  firstLink,
  noteAfterLink,
  resultText,
  taskFromInput,
} from "../src/app.ts";

const client = {} as unknown as WebClient;

test("firstLink reads Slack's angled links and bare links", () => {
  assert.equal(firstLink("see <https://example.org/a|the page> next"), "https://example.org/a");
  assert.equal(firstLink("see <https://example.org/b> next"), "https://example.org/b");
  assert.equal(firstLink("see https://example.org/c next"), "https://example.org/c");
  assert.equal(firstLink("nothing here"), null);
});

test("noteAfterLink keeps only the text a person typed", () => {
  assert.equal(noteAfterLink("https://example.org/a please review"), "please review");
  assert.equal(noteAfterLink("<https://example.org/a|A> ready"), "ready");
  assert.equal(noteAfterLink("https://example.org/a"), "");
});

test("resultText reports success with a task link", () => {
  const text = resultText({
    ok: true,
    task: { id: "t", title: "Buy milk", link: "https://tasks.google.com/task/1" },
    via: "direct",
  });
  assert.match(text, /Added to your TimeSlay inbox: \*Buy milk\*/);
  assert.match(text, /<https:\/\/tasks\.google\.com\/task\/1\|Open the task>/);
  assert.match(text, /via direct/);
});

test("resultText reports a failure with the whole trace", () => {
  const text = resultText({
    ok: false,
    error: "Could not read the link",
    trace: "Steps:\nasking timeslay-window.sock",
  });
  assert.match(text, /:x: Could not read the link/);
  assert.match(text, /timeslay-window\.sock/);
});

test("contextFor builds the Slack link and author", () => {
  const context = contextFor(
    { user: "U1", ts: "1700000000.000100" },
    "C2",
    "1700000000.000100",
    "T1",
    "#general",
  );
  assert.equal(context.author, "<@U1>");
  assert.equal(context.channel, "#general");
  assert.equal(context.permalink, "https://app.slack.com/client/T1/C2/1700000000.000100");
  assert.equal(context.sentAt, "2023-11-14T22:13:20.000Z");
});

test("a typed title wins over the note's first line", async () => {
  const task = await taskFromInput(
    client,
    "https://example.org/article",
    "some note",
    "My title",
  );
  assert.equal(task.title, "My title");
  assert.equal(task.url, "https://example.org/article");
  assert.equal(task.source, "slack");
  assert.match(task.description, /some note/);
});

test("with no title, the note's first line names the task", async () => {
  const task = await taskFromInput(
    client,
    "https://example.org/article",
    "Review the pull request",
    "",
  );
  assert.equal(task.title, "Review the pull request");
});

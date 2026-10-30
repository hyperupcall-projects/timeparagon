import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTask,
  buildTaskDescription,
  messageBody,
  parseSlackLink,
  sentAt,
  slackWebLink,
  taskTitle,
} from "../src/format.ts";

test("messageBody keeps the text, attachments and files", () => {
  const body = messageBody({
    text: "Please fix this",
    attachments: [{ title: "Deploy failed", text: "job 42" }],
    files: [{ name: "log.txt", permalink: "https://files.slack.com/log.txt" }],
  });
  assert.match(body, /Please fix this/);
  assert.match(body, /\[Attachment\]/);
  assert.match(body, /Deploy failed/);
  assert.match(body, /\[File\] log\.txt/);
  assert.match(body, /https:\/\/files\.slack\.com\/log\.txt/);
});

test("a blocks-only message keeps its blocks", () => {
  const body = messageBody({ blocks: [{ type: "section" }] });
  assert.match(body, /"type": "section"/);
});

test("an empty message still has a body", () => {
  assert.equal(messageBody({}), "(this message has no text)");
});

test("taskTitle uses the first line and strips Slack markup", () => {
  assert.equal(
    taskTitle("<https://example.org|Read this> please\nmore"),
    "Read this please",
  );
  assert.equal(taskTitle("<@U123> ping <#C1|general>"), "@U123 ping #general");
});

test("taskTitle is never empty and is bounded", () => {
  assert.equal(taskTitle("", "Fallback"), "Fallback");
  assert.equal(Array.from(taskTitle("a".repeat(500))).length, 120);
});

test("sentAt converts a Slack timestamp", () => {
  assert.equal(sentAt("1700000000.000100"), "2023-11-14T22:13:20.000Z");
  assert.equal(sentAt(undefined), "");
  assert.equal(sentAt("nonsense"), "");
});

test("slackWebLink opens the message in the Slack client", () => {
  assert.equal(
    slackWebLink("T1", "C2", "1700000000.000100"),
    "https://app.slack.com/client/T1/C2/1700000000.000100",
  );
});

test("parseSlackLink reads a channel permalink", () => {
  assert.deepEqual(
    parseSlackLink("https://acme.slack.com/archives/C01234ABCDE/p1700000000000100"),
    { channel: "C01234ABCDE", ts: "1700000000.000100" },
  );
});

test("parseSlackLink reads an app.slack.com client link", () => {
  assert.deepEqual(
    parseSlackLink("https://app.slack.com/client/T1/C2/1700000000.000100"),
    { team: "T1", channel: "C2", ts: "1700000000.000100" },
  );
});

test("parseSlackLink reads a slack:// deep link", () => {
  assert.deepEqual(
    parseSlackLink("slack://channel?team=T1&id=C2&message=1700000000.000100"),
    { team: "T1", channel: "C2", ts: "1700000000.000100" },
  );
});

test("parseSlackLink rejects anything else", () => {
  assert.equal(parseSlackLink("https://example.org/article"), null);
});

test("buildTaskDescription puts the metadata, link and text together", () => {
  const description = buildTaskDescription("Hello", {
    author: "<@U1>",
    channel: "#general",
    sentAt: "2023-11-14T22:13:20.000Z",
    permalink: "https://app.slack.com/client/T1/C2/1.2",
  });
  assert.match(description, /^From: <@U1>/);
  assert.match(description, /Channel: #general/);
  assert.match(description, /Link: https:\/\/app\.slack\.com\/client\/T1\/C2\/1\.2/);
  assert.match(description, /Hello$/);
});

test("buildTask derives the url, title and source from the context", () => {
  const task = buildTask("Please ship it", {
    permalink: "https://app.slack.com/client/T1/C2/1.2",
  });
  assert.equal(task.url, "https://app.slack.com/client/T1/C2/1.2");
  assert.equal(task.title, "Please ship it");
  assert.equal(task.source, "slack");
  assert.match(task.description, /Please ship it/);
});

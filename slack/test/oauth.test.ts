import assert from "node:assert/strict";
import test from "node:test";
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { IncomingMessage, ServerResponse, RequestListener } from "node:http";
import type { WebClient } from "@slack/web-api";
import type { App } from "@slack/bolt";
import { Installations, authorizeInstallation, type Installation } from "../src/installations.ts";
import { oauthHandler, BOT_SCOPES } from "../src/oauth.ts";
import { actorId, installOwnerGuard } from "../src/app.ts";
import { loadConfig } from "../src/config.ts";

const entry: Installation = { teamId: "T1", ownerUserId: "U1", botToken: "bot-one", botUserId: "UB1", botId: "B1" };
const config = { clientId: "client", clientSecret: "secret", redirectUri: "https://example.org/slack/oauth/callback", port: 3000, installationsFile: "unused" };
function fixture(t: test.TestContext) {
  const dir = mkdtempSync(join(tmpdir(), "timeslay-oauth-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const path = join(dir, "installations.json");
  return { path, store: new Installations(path, "client") };
}
async function request(handler: RequestListener, url: string, cookie = "", method = "GET") {
  const headers: Record<string, string> = {};
  const response = { statusCode: 200, body: "", setHeader(name: string, value: string) { headers[name] = value; },
    end(body: string) { this.body = body; } };
  await handler({ url, method, headers: { cookie } } as IncomingMessage, response as unknown as ServerResponse);
  return { ...response, headers };
}
async function begin(handler: RequestListener) {
  const res = await request(handler, "/slack/install?key=private");
  assert.equal(res.statusCode, 302);
  const url = new URL(res.headers.Location);
  assert.equal(url.origin, "https://slack.com");
  assert.equal(url.searchParams.get("redirect_uri"), config.redirectUri);
  assert.equal(url.searchParams.get("scope"), BOT_SCOPES.join(","));
  return { state: url.searchParams.get("state")!, cookie: res.headers["Set-Cookie"].split(";")[0] };
}
function slackStub(overrides = {}) {
  const exchanges: unknown[] = [];
  const client = { oauth: { v2: { access: async (args: unknown) => {
    exchanges.push(args);
    return { ok: true, token_type: "bot", team: { id: "T1" }, authed_user: { id: "U1" }, access_token: "bot-one", bot_user_id: "UB1", ...overrides };
  } } }, auth: { test: async () => ({ ok: true, team_id: "T1", user_id: "UB1", bot_id: "B1" }) } } as unknown as WebClient;
  return { client, exchanges };
}

test("installation store separates workspace tokens, persists privately, and protects ownership", (t) => {
  const { store, path } = fixture(t);
  store.save(entry);
  store.save({ ...entry, teamId: "T2", ownerUserId: "U2", botToken: "bot-two" });
  assert.equal(store.get("T1")?.botToken, "bot-one");
  assert.equal(store.get("T2")?.botToken, "bot-two");
  assert.equal(store.get("missing"), undefined);
  assert.throws(() => store.save({ ...entry, ownerUserId: "U2" }), /different Slack user/);
  assert.equal(statSync(path).mode & 0o777, 0o600);
  assert.equal(new Installations(path, "client").get("T2")?.ownerUserId, "U2");
  assert.throws(() => new Installations(path, "other-client"), /Could not load/);
  writeFileSync(path, "broken");
  assert.throws(() => new Installations(path, "client"), /Could not load/);
});

test("OAuth requires private link, matching cookie, single-use state, and saves verified identity", async (t) => {
  const { store, path } = fixture(t);
  const { client, exchanges } = slackStub();
  const handler = oauthHandler(config, store, "private", client);
  assert.equal((await request(handler, "/slack/install")).statusCode, 403);
  assert.equal((await request(handler, "/slack/install?key=wrong")).statusCode, 403);
  const { state, cookie } = await begin(handler);
  const callback = `/slack/oauth/callback?state=${state}&code=code`;
  assert.equal((await request(handler, callback)).statusCode, 400);
  assert.equal(exchanges.length, 0);
  const res = await request(handler, callback, cookie);
  assert.equal(res.statusCode, 200);
  assert.deepEqual(store.get("T1"), entry);
  assert.deepEqual(exchanges[0], { client_id: "client", client_secret: "secret", redirect_uri: config.redirectUri, code: "code" });
  assert.match(readFileSync(path, "utf8"), /T1/);
  assert.equal((await request(handler, callback, cookie)).statusCode, 400);
  assert.equal(exchanges.length, 1);
});

test("expired state, denial, rotating bot tokens, and org installations never save credentials", async (t) => {
  const { store } = fixture(t);
  let now = 100;
  const stub = slackStub();
  const handler = oauthHandler(config, store, "private", stub.client, () => now);
  const expired = await begin(handler);
  now += 600_001;
  assert.equal((await request(handler, `/slack/oauth/callback?state=${expired.state}&code=x`, expired.cookie)).statusCode, 400);
  assert.equal(stub.exchanges.length, 0);
  const denied = await begin(handler);
  assert.equal((await request(handler, `/slack/oauth/callback?state=${denied.state}&error=access_denied`, denied.cookie)).statusCode, 400);
  for (const overrides of [{ refresh_token: "refresh", expires_in: 100 }, { is_enterprise_install: true }, { authed_user: {} }]) {
    const bad = oauthHandler(config, store, "private", slackStub(overrides).client);
    const { state, cookie } = await begin(bad);
    assert.equal((await request(bad, `/slack/oauth/callback?state=${state}&code=x`, cookie)).statusCode, 400);
  }
  assert.equal(store.get("T1"), undefined);
});

test("OAuth errors do not disclose credentials or replace existing installations", async (t) => {
  const { store } = fixture(t);
  store.save(entry);
  const stub = slackStub({ authed_user: { id: "attacker" } });
  const handler = oauthHandler(config, store, "private", stub.client);
  const { state, cookie } = await begin(handler);
  const response = await request(handler, `/slack/oauth/callback?state=${state}&code=x`, cookie);
  assert.equal(response.statusCode, 500);
  assert.doesNotMatch(response.body, /bot-one|attacker|secret/);
  assert.deepEqual(store.get("T1"), entry);
});

test("owner guard blocks commands, shortcuts, forms, and events before any listener", async () => {
  type Args = { body: unknown; context: { ownerUserId?: string }; next: () => Promise<void>; ack: () => Promise<void>; respond: () => Promise<void> };
  let guard!: (args: Args) => Promise<void>;
  const app = { use(fn: typeof guard) { guard = fn; } } as unknown as App;
  installOwnerGuard(app, true);
  for (const body of [{ user_id: "other" }, { user: { id: "other" }, message: { user: "U1" } }, { type: "view_submission", user: { id: "other" } }, { event: { user: "other" } }, {}]) {
    let called = false, acknowledged = false;
    await guard({ body, context: { ownerUserId: "U1" }, next: async () => { called = true; }, ack: async () => { acknowledged = true; }, respond: async () => {} });
    assert.equal(called, false);
    assert.equal(acknowledged, true);
  }
  for (const body of [{ user_id: "U1" }, { user: { id: "U1" }, message: { user: "other" } }, { event: { user: "U1" } }]) {
    let called = false;
    await guard({ body, context: { ownerUserId: "U1" }, next: async () => { called = true; }, ack: async () => {}, respond: async () => {} });
    assert.equal(called, true);
    assert.equal(actorId(body), "U1");
  }
});

test("configuration preserves legacy mode and validates OAuth setup", () => {
  const legacy = loadConfig({ SLACK_APP_TOKEN: "xapp-local", SLACK_BOT_TOKEN: "bot-local" });
  assert.equal(legacy.botToken, "bot-local");
  assert.equal(legacy.oauth, undefined);
  const env = { SLACK_APP_TOKEN: "xapp-local", SLACK_REDIRECT_URI: config.redirectUri, SLACK_CLIENT_ID: "client", SLACK_CLIENT_SECRET: "secret" };
  assert.equal(loadConfig(env).oauth?.port, 3000);
  assert.equal(loadConfig(env).botToken, "");
  assert.throws(() => loadConfig({ ...env, SLACK_REDIRECT_URI: "http://example.org/slack/oauth/callback" }));
  assert.throws(() => loadConfig({ ...env, SLACK_OAUTH_PORT: "0" }));
  assert.throws(() => loadConfig({ ...env, SLACK_CLIENT_SECRET: "" }));
});

test("authorization selects workspace tokens and rejects unknown or org-wide installations", async (t) => {
  const { store } = fixture(t);
  store.save(entry);
  store.save({ ...entry, teamId: "T2", ownerUserId: "U2", botToken: "bot-two" });
  const authorize = authorizeInstallation(store);
  const source = { teamId: "T1", userId: "U1", enterpriseId: undefined, isEnterpriseInstall: false };
  assert.equal((await authorize(source)).botToken, "bot-one");
  assert.equal((await authorize({ ...source, teamId: "T2" })).botToken, "bot-two");
  await assert.rejects(authorize({ ...source, teamId: "unknown" }));
  await assert.rejects(authorize({ ...source, isEnterpriseInstall: true }));
});

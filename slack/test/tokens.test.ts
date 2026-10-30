import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { loadConfig } from "../src/config.ts";
import { AppTokens } from "../src/tokens.ts";

async function fixture(t: { after: (fn: () => Promise<void>) => void }) {
  const dir = await mkdtemp(join(tmpdir(), "slack-tokens-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return {
    clientId: "client-id",
    clientSecret: "client-secret",
    refreshToken: "initial-refresh",
    tokenFile: join(dir, "tokens.json"),
  };
}

function response(n: number) {
  return Response.json({
    ok: true, token_type: "app_level", access_token: `access-${n}`,
    refresh_token: `refresh-${n}`, expires_in: 43200,
  });
}

test("refresh sends OAuth credentials and persists both replacement tokens privately", async (t) => {
  const config = await fixture(t);
  const requests: URLSearchParams[] = [];
  let now = 1_000_000;
  const fetchToken: typeof fetch = async (url, init) => {
    assert.equal(url, "https://slack.com/api/oauth.v2.access");
    assert.equal(init?.method, "POST");
    const body = init?.body as URLSearchParams;
    requests.push(body);
    assert.equal(body.get("client_id"), config.clientId);
    assert.equal(body.get("client_secret"), config.clientSecret);
    assert.equal(body.get("grant_type"), "refresh_token");
    return response(requests.length);
  };
  const tokens = new AppTokens(config, fetchToken, () => now);
  assert.deepEqual(await Promise.all([tokens.getToken(), tokens.getToken()]), ["access-1", "access-1"]);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].get("refresh_token"), "initial-refresh");
  const saved = JSON.parse(await readFile(config.tokenFile, "utf8"));
  assert.equal(saved.accessToken, "access-1");
  assert.equal(saved.refreshToken, "refresh-1");
  assert.equal(saved.expiresAt, now + 43200_000);
  assert.equal((await stat(config.tokenFile)).mode & 0o777, 0o600);

  // Restart uses the persisted pair rather than consuming the original token.
  const restarted = new AppTokens(config, fetchToken, () => now);
  assert.equal(await restarted.getToken(), "access-1");
  assert.equal(requests.length, 1);
  now = saved.refreshAt;
  assert.equal(await restarted.getToken(), "access-2");
  assert.equal(requests[1].get("refresh_token"), "refresh-1");
  assert.equal(JSON.parse(await readFile(config.tokenFile, "utf8")).refreshToken, "refresh-2");
});

test("OAuth failures neither overwrite cached tokens nor expose response secrets", async (t) => {
  const config = await fixture(t);
  let now = 0;
  const tokens = new AppTokens(config, async () => response(1), () => now);
  await tokens.getToken();
  const before = await readFile(config.tokenFile, "utf8");
  now = 43200_000;
  const failing = new AppTokens(config, async () => Response.json({
    ok: false, error: "secret-that-must-not-appear",
  }), () => now);
  await assert.rejects(failing.getToken(), (error: Error) => {
    assert.match(error.message, /oauth.v2.access/);
    assert.doesNotMatch(error.message, /secret-that-must-not-appear/);
    return true;
  });
  assert.equal(await readFile(config.tokenFile, "utf8"), before);
});

test("a corrupt cache fails closed instead of reusing a consumed refresh token", async (t) => {
  const config = await fixture(t);
  await writeFile(config.tokenFile, "broken-json");
  const tokens = new AppTokens(config, async () => {
    assert.fail("must not exchange the original token when the cache is unreadable");
  });
  await assert.rejects(tokens.getToken(), /Could not load Slack app token cache/);
});

test("a failed save retries the replacement pair without consuming another refresh token", async (t) => {
  const config = await fixture(t);
  let calls = 0;
  const tokens = new AppTokens(config, async () => {
    calls++;
    // Simulate a destination that becomes unwritable during the exchange.
    await mkdir(config.tokenFile);
    return response(1);
  });
  await assert.rejects(tokens.getToken(), /Could not save rotated Slack tokens/);
  await rm(config.tokenFile, { recursive: true });
  assert.equal(await tokens.getToken(), "access-1");
  assert.equal(calls, 1);
  assert.equal(JSON.parse(await readFile(config.tokenFile, "utf8")).refreshToken, "refresh-1");
});

test("bot tokens are not accepted as an app-level rotation response", async (t) => {
  const config = await fixture(t);
  const tokens = new AppTokens(config, async () => Response.json({
    ok: true, token_type: "bot", access_token: "bot", refresh_token: "refresh", expires_in: 43200,
  }));
  await assert.rejects(tokens.getToken(), /valid app-level token pair/);
});

test("rotating app tokens require refresh and client credentials; legacy tokens still work", () => {
  const env = { SLACK_BOT_TOKEN: "bot", SLACK_APP_TOKEN: "xoxe.xapp-example" };
  assert.throws(() => loadConfig(env), /SLACK_APP_REFRESH_TOKEN/);
  assert.throws(() => loadConfig({ ...env, SLACK_APP_REFRESH_TOKEN: "refresh" }), /SLACK_CLIENT_ID/);
  assert.throws(() => loadConfig({ ...env, SLACK_APP_REFRESH_TOKEN: "refresh", SLACK_CLIENT_ID: "id" }), /SLACK_CLIENT_SECRET/);
  const configured = loadConfig({ ...env, SLACK_APP_REFRESH_TOKEN: "refresh", SLACK_CLIENT_ID: "id", SLACK_CLIENT_SECRET: "secret" });
  assert.equal(configured.appRotation?.refreshToken, "refresh");
  assert.equal(loadConfig({ ...env, SLACK_APP_TOKEN: "xapp-legacy" }).appRotation, undefined);
});

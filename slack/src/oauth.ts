import { randomBytes, timingSafeEqual } from "node:crypto";
import { createServer, type RequestListener } from "node:http";
import { WebClient } from "@slack/web-api";
import type { Config } from "./config.ts";
import { Installations } from "./installations.ts";

export const BOT_SCOPES = ["chat:write", "commands", "im:write", "channels:history",
  "groups:history", "im:history", "mpim:history", "users:read"];
const COOKIE = "timeslay_oauth_state";
const random = () => randomBytes(32).toString("hex");
function matches(a: string, b: string): boolean {
  const left = Buffer.from(a), right = Buffer.from(b);
  return left.length === right.length && timingSafeEqual(left, right);
}

type OAuthConfig = NonNullable<Config["oauth"]>;
/** The private setup key protects a bridge that writes to one person's local inbox. */
export function oauthHandler(config: OAuthConfig, store: Installations, setupKey: string,
  client = new WebClient(undefined, { retryConfig: { retries: 0 }, timeout: 30_000 }),
  now = Date.now): RequestListener {
  const states = new Map<string, number>();
  return async (req, res) => {
    res.setHeader("Cache-Control", "no-store");
    res.setHeader("Referrer-Policy", "no-referrer");
    res.setHeader("Content-Type", "text/plain; charset=utf-8");
    res.setHeader("X-Content-Type-Options", "nosniff");
    const finish = (status: number, message: string) => { res.statusCode = status; res.end(message); };
    try {
      const url = new URL(req.url ?? "/", "http://localhost");
      if (req.method !== "GET") { finish(405, "GET required."); return; }
      if (url.pathname === "/slack/install") {
        if (!matches(url.searchParams.get("key") ?? "", setupKey)) {
          finish(403, "Open the private connection link printed by your TimeSlay bridge."); return;
        }
        for (const [state, expires] of states) if (expires <= now()) states.delete(state);
        if (states.size >= 100) { finish(429, "Too many pending connections. Try again later."); return; }
        const state = random();
        states.set(state, now() + 600_000);
        res.setHeader("Set-Cookie", `${COOKIE}=${state}; HttpOnly; Secure; SameSite=Lax; Path=/slack; Max-Age=600`);
        const authorize = new URL("https://slack.com/oauth/v2/authorize");
        authorize.search = new URLSearchParams({ client_id: config.clientId,
          scope: BOT_SCOPES.join(","), redirect_uri: config.redirectUri, state }).toString();
        res.setHeader("Location", authorize.href);
        finish(302, "Continue to Slack to connect a workspace."); return;
      }
      if (url.pathname !== "/slack/oauth/callback") { finish(404, "Not found."); return; }
      const state = url.searchParams.get("state") ?? "";
      const cookie = req.headers.cookie?.split(";").map((v) => v.trim())
        .find((v) => v.startsWith(`${COOKIE}=`))?.slice(COOKIE.length + 1) ?? "";
      const expires = states.get(state);
      if (!expires || expires <= now() || !matches(state, cookie)) {
        finish(400, "Invalid or expired connection. Open your private connection link again."); return;
      }
      states.delete(state); // consume before exchanging the single-use authorization code
      res.setHeader("Set-Cookie", `${COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/slack; Max-Age=0`);
      if (url.searchParams.has("error")) { finish(400, "Slack connection was cancelled or denied."); return; }
      const code = url.searchParams.get("code");
      if (!code) { finish(400, "Missing authorization code."); return; }
      const result = await client.oauth.v2.access({ client_id: config.clientId,
        client_secret: config.clientSecret, redirect_uri: config.redirectUri, code });
      if (!result.ok || result.token_type !== "bot" || result.is_enterprise_install || !result.team?.id ||
          !result.authed_user?.id || !result.access_token || !result.bot_user_id ||
          result.refresh_token || result.expires_in) {
        finish(400, "A workspace bot installation without bot token rotation is required."); return;
      }
      const auth = await client.auth.test({ token: result.access_token });
      if (!auth.ok || auth.team_id !== result.team.id || !auth.bot_id || auth.user_id !== result.bot_user_id) {
        throw new Error("Unexpected bot identity");
      }
      store.save({ teamId: result.team.id, ownerUserId: result.authed_user.id,
        botToken: result.access_token, botUserId: result.bot_user_id, botId: auth.bot_id });
      finish(200, "Workspace connected to your TimeSlay inbox. Only the Slack account you just authorized can add tasks. You can close this tab and connect another workspace using the same private link.");
    } catch {
      // SDK errors can contain request credentials: never echo or log them.
      finish(500, "Could not connect the workspace. Check client credentials, bot rotation settings, and installation file permissions, then retry the private link. Existing workspace ownership cannot be replaced by another Slack account.");
    }
  };
}

export async function startOAuth(config: OAuthConfig, store: Installations) {
  const key = random();
  const server = createServer(oauthHandler(config, store, key));
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(config.port, "127.0.0.1", () => { server.off("error", reject); resolve(); });
  });
  const link = new URL("/slack/install", config.redirectUri);
  link.searchParams.set("key", key);
  console.log(`Connect a workspace (private link; do not share): ${link.href}`);
  console.log(`Forward HTTPS /slack/* to http://127.0.0.1:${config.port}.`);
  return server;
}

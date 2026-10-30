import { chmod, readFile, rename, unlink, writeFile } from "node:fs/promises";
import type { Config } from "./config.ts";

type TokenPair = {
  accessToken: string;
  refreshToken: string;
  expiresAt: number;
  refreshAt: number;
  clientId: string;
};

function isPair(value: unknown): value is TokenPair {
  const pair = value as TokenPair | null;
  return !!pair && typeof pair.accessToken === "string" && !!pair.accessToken &&
    typeof pair.refreshToken === "string" && !!pair.refreshToken &&
    typeof pair.clientId === "string" &&
    Number.isFinite(pair.expiresAt) && Number.isFinite(pair.refreshAt) &&
    pair.refreshAt < pair.expiresAt;
}

/** One manager per process: concurrent callers share a single refresh. */
export class AppTokens {
  private pair?: TokenPair;
  private loaded = false;
  private dirty = false;
  private pending?: Promise<string>;
  private readonly rotation: NonNullable<Config["appRotation"]>;
  private readonly fetchToken: typeof fetch;
  private readonly now: () => number;

  constructor(
    rotation: NonNullable<Config["appRotation"]>,
    fetchToken: typeof fetch = fetch,
    now: () => number = Date.now,
  ) {
    this.rotation = rotation;
    this.fetchToken = fetchToken;
    this.now = now;
  }

  get refreshAt(): number {
    return this.pair?.refreshAt ?? this.now();
  }

  getToken(): Promise<string> {
    if (!this.pending) {
      this.pending = this.loadOrRefresh().finally(() => { this.pending = undefined; });
    }
    return this.pending;
  }

  private async persist(): Promise<void> {
    const path = this.rotation.tokenFile;
    const temp = `${path}.${process.pid}.tmp`;
    try {
      await writeFile(temp, JSON.stringify(this.pair) + "\n", { mode: 0o600, flag: "wx" });
      await rename(temp, path);
      this.dirty = false;
    } catch {
      throw new Error("Could not save rotated Slack tokens. Check SLACK_APP_TOKEN_FILE permissions; keep this process running to retry saving.");
    } finally {
      await unlink(temp).catch(() => {});
    }
  }

  private async loadOrRefresh(): Promise<string> {
    if (!this.loaded) {
      try {
        const saved: unknown = JSON.parse(await readFile(this.rotation.tokenFile, "utf8"));
        if (!isPair(saved) || saved.clientId !== this.rotation.clientId) {
          throw new Error("Invalid or mismatched Slack app token cache.");
        }
        await chmod(this.rotation.tokenFile, 0o600);
        this.pair = saved;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
          throw new Error("Could not load Slack app token cache. Check SLACK_APP_TOKEN_FILE.");
        }
      }
      this.loaded = true;
    }

    // If a disk write failed after Slack consumed the refresh token, retry
    // saving that same replacement pair before making another exchange.
    if (this.dirty) await this.persist();
    if (this.pair && this.now() < this.pair.refreshAt) return this.pair.accessToken;

    const requestedAt = this.now();
    let result: Record<string, unknown>;
    try {
      const response = await this.fetchToken("https://slack.com/api/oauth.v2.access", {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          client_id: this.rotation.clientId,
          client_secret: this.rotation.clientSecret,
          grant_type: "refresh_token",
          refresh_token: this.pair?.refreshToken ?? this.rotation.refreshToken,
        }),
        signal: AbortSignal.timeout(30_000),
      });
      if (!response.ok) throw new Error();
      result = await response.json() as Record<string, unknown>;
    } catch {
      // Never include the request, response body or credentials in errors.
      throw new Error("Slack oauth.v2.access request failed.");
    }
    if (result.ok !== true || result.token_type !== "app_level" ||
      typeof result.access_token !== "string" || !result.access_token ||
      typeof result.refresh_token !== "string" || !result.refresh_token ||
      typeof result.expires_in !== "number" || !Number.isFinite(result.expires_in) || result.expires_in <= 0) {
      throw new Error("Slack oauth.v2.access did not return a valid app-level token pair. Check the app refresh token and client credentials.");
    }
    const lifetime = result.expires_in * 1000;
    this.pair = {
      accessToken: result.access_token,
      refreshToken: result.refresh_token,
      expiresAt: requestedAt + lifetime,
      refreshAt: requestedAt + lifetime - Math.min(300_000, lifetime / 2),
      clientId: this.rotation.clientId,
    };
    this.dirty = true;
    await this.persist();
    return this.pair.accessToken;
  }
}

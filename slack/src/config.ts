import { fileURLToPath } from "node:url";

export type Config = {
  botToken: string;
  allowedUserId?: string;
  oauth?: {
    clientId: string;
    clientSecret: string;
    redirectUri: string;
    port: number;
    installationsFile: string;
  };
  appToken: string;
  appRotation?: {
    refreshToken: string;
    clientId: string;
    clientSecret: string;
    tokenFile: string;
  };
  timeslayBin: string;
  timeslayTimeoutMs: number;
};

function required(env: NodeJS.ProcessEnv, name: string): string {
  const value = env[name]?.trim();
  if (!value) {
    throw new Error(
      `Missing ${name}. Copy .env.example to .env and fill in the Slack tokens.`,
    );
  }
  return value;
}

function timeout(env: NodeJS.ProcessEnv): number {
  const raw = env.TIMESLAY_TIMEOUT_MS?.trim();
  if (!raw) return 30_000;
  const value = Number(raw);
  return Number.isFinite(value) && value > 0 ? value : 30_000;
}

/**
 * Reads the configuration from the environment. Kept as a function (rather
 * than a module-level constant) so importing this file never fails when the
 * Slack tokens are absent, which keeps the helpers unit-testable.
 */
export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const appToken = required(env, "SLACK_APP_TOKEN");
  const refreshToken = env.SLACK_APP_REFRESH_TOKEN?.trim();
  if (appToken.startsWith("xoxe.") && !refreshToken) {
    throw new Error("Rotating app tokens require SLACK_APP_REFRESH_TOKEN.");
  }
  const redirectUri = env.SLACK_REDIRECT_URI?.trim();
  let oauth: Config["oauth"];
  if (redirectUri) {
    const url = new URL(redirectUri);
    if (url.protocol !== "https:" || url.pathname !== "/slack/oauth/callback" ||
        url.search || url.hash || url.username || url.password) {
      throw new Error("SLACK_REDIRECT_URI must be https://your-host/slack/oauth/callback.");
    }
    const port = Number(env.SLACK_OAUTH_PORT ?? "3000");
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      throw new Error("SLACK_OAUTH_PORT must be between 1 and 65535.");
    }
    oauth = {
      clientId: required(env, "SLACK_CLIENT_ID"),
      clientSecret: required(env, "SLACK_CLIENT_SECRET"),
      redirectUri, port,
      installationsFile: env.SLACK_INSTALLATIONS_FILE?.trim() ||
        fileURLToPath(new URL("../.slack-installations.json", import.meta.url)),
    };
  }
  return {
    botToken: oauth ? "" : required(env, "SLACK_BOT_TOKEN"),
    allowedUserId: env.SLACK_ALLOWED_USER_ID?.trim() || undefined,
    oauth,
    appToken,
    ...(refreshToken ? {
      appRotation: {
        refreshToken,
        clientId: required(env, "SLACK_CLIENT_ID"),
        clientSecret: required(env, "SLACK_CLIENT_SECRET"),
        tokenFile: env.SLACK_APP_TOKEN_FILE?.trim() ||
          fileURLToPath(new URL("../.slack-app-tokens.json", import.meta.url)),
      },
    } : {}),
    timeslayBin: env.TIMESLAY_BIN?.trim() || "timeslay",
    timeslayTimeoutMs: timeout(env),
  };
}

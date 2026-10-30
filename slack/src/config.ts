export type Config = {
  botToken: string;
  appToken: string;
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
  return {
    botToken: required(env, "SLACK_BOT_TOKEN"),
    appToken: required(env, "SLACK_APP_TOKEN"),
    timeslayBin: env.TIMESLAY_BIN?.trim() || "time-slay",
    timeslayTimeoutMs: timeout(env),
  };
}

import { chmodSync, readFileSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import type { Authorize } from "@slack/bolt";

export type Installation = {
  teamId: string;
  ownerUserId: string;
  botToken: string;
  botUserId: string;
  botId: string;
};

function valid(value: unknown): value is Installation {
  const v = value as Installation | null;
  return !!v && [v.teamId, v.ownerUserId, v.botToken, v.botUserId, v.botId]
    .every((s) => typeof s === "string" && s.length > 0);
}

/** One bridge process owns this file. Writes complete before new tokens are used. */
export class Installations {
  private entries: Installation[] = [];
  private readonly path: string;
  private readonly clientId: string;
  constructor(path: string, clientId: string) {
    this.path = path;
    this.clientId = clientId;
    try {
      const data = JSON.parse(readFileSync(path, "utf8"));
      if (data.clientId !== clientId || !Array.isArray(data.installations) ||
          !data.installations.every(valid)) throw new Error("Invalid installation file");
      this.entries = data.installations;
      chmodSync(path, 0o600);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
        throw new Error("Could not load Slack installations; check SLACK_INSTALLATIONS_FILE and client ID.");
      }
    }
  }

  get(teamId: string): Installation | undefined {
    return this.entries.find((entry) => entry.teamId === teamId);
  }

  save(entry: Installation): void {
    if (!valid(entry)) throw new Error("Invalid Slack installation");
    const previous = this.get(entry.teamId);
    if (previous && previous.ownerUserId !== entry.ownerUserId) {
      throw new Error("This workspace is already connected by a different Slack user.");
    }
    const next = [...this.entries.filter((v) => v.teamId !== entry.teamId), entry];
    const temp = `${this.path}.${randomUUID()}.tmp`;
    try {
      writeFileSync(temp, JSON.stringify({ clientId: this.clientId, installations: next }) + "\n",
        { mode: 0o600, flag: "wx" });
      renameSync(temp, this.path);
      this.entries = next;
    } finally {
      try { unlinkSync(temp); } catch { /* renamed, or no temporary file created */ }
    }
  }
}

/** Resolve credentials only for the workspace named by Slack's incoming payload. */
export function authorizeInstallation(store: Installations): Authorize<boolean> {
  return async ({ teamId, isEnterpriseInstall }) => {
    const entry = !isEnterpriseInstall && teamId && store.get(teamId);
    if (!entry) throw new Error("Slack workspace is not connected to TimeSlay.");
    return { ...entry };
  };
}

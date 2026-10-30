import { spawn } from "node:child_process";
import { endianness } from "node:os";

export type AddRequest = {
  command?: string;
  url: string;
  title: string;
  description: string;
  source?: string;
  /** The day the task is due, as `YYYY-MM-DD`. */
  due?: string;
  /** When the task must be finished, as `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`. */
  deadline?: string;
};

export type TaskInfo = {
  id: string;
  title: string;
  link: string;
};

export type AddResult = {
  ok: boolean;
  task?: TaskInfo;
  error?: string;
  trace?: string;
  via?: string;
};

export type AddOptions = {
  /** The time-slay binary. Defaults to TIMESLAY_BIN or `time-slay` on PATH. */
  bin?: string;
  timeoutMs?: number;
};

function binary(options: AddOptions): string {
  return options.bin?.trim() || process.env.TIMESLAY_BIN?.trim() || "time-slay";
}

function timeout(options: AddOptions): number {
  if (options.timeoutMs && options.timeoutMs > 0) return options.timeoutMs;
  const raw = Number(process.env.TIMESLAY_TIMEOUT_MS);
  return Number.isFinite(raw) && raw > 0 ? raw : 30_000;
}

/** The four-byte length prefix both Chrome and Firefox use. */
export function encodeNativeMessage(value: unknown): Buffer {
  const body = Buffer.from(JSON.stringify(value), "utf8");
  const header = Buffer.allocUnsafe(4);
  if (endianness() === "LE") {
    header.writeUInt32LE(body.length, 0);
  } else {
    header.writeUInt32BE(body.length, 0);
  }
  return Buffer.concat([header, body]);
}

/** Reads one native message from the front of `buffer`, or null if incomplete. */
export function decodeNativeMessage(buffer: Buffer): unknown | null {
  if (buffer.length < 4) return null;
  const length =
    endianness() === "LE"
      ? buffer.readUInt32LE(0)
      : buffer.readUInt32BE(0);
  if (buffer.length < 4 + length) return null;
  const body = buffer.subarray(4, 4 + length).toString("utf8");
  return JSON.parse(body) as unknown;
}

function isAddResult(value: unknown): value is AddResult {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { ok?: unknown }).ok === "boolean"
  );
}

/**
 * Runs `time-slay --webextension` for one message. The binary is the same
 * native host the browser uses, so the Slack app and the extension share one
 * code path (including the Gmail-aware link handling and the running-app IPC).
 */
export async function addToInbox(
  request: AddRequest,
  options: AddOptions = {},
): Promise<AddResult> {
  const bin = binary(options);
  const limit = timeout(options);
  const payload = encodeNativeMessage({
    command: "add-to-inbox",
    ...request,
  });

  return await new Promise<AddResult>((resolve) => {
    let stdout = Buffer.alloc(0);
    let stderr = "";
    let settled = false;
    let timer: NodeJS.Timeout | undefined;

    const finish = (result: AddResult) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      resolve(result);
    };

    const failure = (error: string, extra = ""): AddResult => ({
      ok: false,
      error,
      trace: [
        "TimeSlay Slack bridge failure",
        `binary: ${bin}`,
        `url: ${request.url}`,
        `timeout: ${limit}ms`,
        extra,
        stderr ? `\n--- time-slay stderr ---\n${stderr}` : "",
      ]
        .filter(Boolean)
        .join("\n"),
    });

    let child;
    try {
      child = spawn(bin, ["--webextension"], {
        stdio: ["pipe", "pipe", "pipe"],
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      finish(failure(`Could not start ${bin}: ${message}`, String(error)));
      return;
    }

    timer = setTimeout(() => {
      child.kill("SIGKILL");
      finish(failure(`Timed out after ${limit}ms waiting for ${bin}`));
    }, limit);

    child.stdout.on("data", (chunk: Buffer) => {
      stdout = Buffer.concat([stdout, chunk]);
    });
    child.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString("utf8");
    });
    child.stdin.on("error", (error) => {
      finish(failure(`Could not write to ${bin}: ${error.message}`, error.stack ?? ""));
    });
    child.on("error", (error) => {
      finish(
        failure(
          `Could not run ${bin}: ${error.message}. Set TIMESLAY_BIN to the time-slay binary.`,
          error.stack ?? "",
        ),
      );
    });
    child.on("close", (code) => {
      if (settled) return;
      if (stdout.length === 0) {
        finish(
          failure(
            `${bin} --webextension exited with code ${code ?? "unknown"} and sent no reply`,
          ),
        );
        return;
      }
      try {
        const reply = decodeNativeMessage(stdout);
        if (!isAddResult(reply)) {
          finish(
            failure(
              `${bin} sent a reply this app does not understand: ${stdout.toString("utf8")}`,
            ),
          );
          return;
        }
        finish(reply);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        finish(
          failure(
            `Could not decode the reply from ${bin}: ${message}`,
            stdout.toString("utf8"),
          ),
        );
      }
    });

    child.stdin.end(payload);
  });
}

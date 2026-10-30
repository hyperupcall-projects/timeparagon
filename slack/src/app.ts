// Add Slack messages to the TimeSlay inbox.
//
// Slack only lets a third-party app add its own entries to the shortcuts menu
// and the message actions menu; it cannot add an item to Slack's built-in
// "Connect apps" area. So this app registers the action in every place Slack
// offers an app:
//
//   * a message shortcut ("Add to inbox") on any message,
//   * a global shortcut ("Add to inbox") that opens a small form,
//   * a slash command (/add-to-inbox <link> [note]),
//   * an App Home tab with instructions.
//
// Every path builds the same task and sends it through the same native host
// (`timeslay --webextension`) the browser extension uses, so the Gmail-aware
// link handling and the running-app IPC are shared.

import { pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

import { App, type RespondFn } from "@slack/bolt";
import type { WebClient } from "@slack/web-api";

import { loadConfig, type Config } from "./config.ts";
import { AppTokens } from "./tokens.ts";
import { Installations, authorizeInstallation } from "./installations.ts";
import { startOAuth } from "./oauth.ts";
import {
  buildTask,
  messageBody,
  parseSlackLink,
  sentAt,
  slackWebLink,
  taskTitle,
  type BuiltTask,
  type SlackMessage,
  type TaskContext,
} from "./format.ts";
import { addToInbox, type AddOptions, type AddResult } from "./timeslay.ts";

/** The message shortcut Slack shows on every message. */
export const MESSAGE_SHORTCUT = "add_to_inbox";
/** The global shortcut that opens the link form. */
export const GLOBAL_SHORTCUT = "add_to_inbox_global";
/** The `callback_id` of the link form opened by the global shortcut. */
export const MODAL_CALLBACK = "add_to_inbox_modal";
/** The slash command. */
export const COMMAND = "/add-to-inbox";

/** Slack rejects a message longer than 4000 characters. */
const MAX_SLACK_TEXT = 3_500;

/**
 * Where the bridge should look for the binary and how long to wait. Comes from
 * the app configuration so both the socket-mode app and any caller agree.
 */
export function addOptions(config: Config): AddOptions {
  return { bin: config.timeslayBin, timeoutMs: config.timeslayTimeoutMs };
}

/** The context lines TimeSlay stores above the message text. */
export function contextFor(
  message: SlackMessage,
  channelId: string,
  messageTs: string,
  teamId: string,
  channelName?: string,
): TaskContext {
  return {
    author: message.user ? `<@${message.user}>` : undefined,
    channel: channelName,
    sentAt: sentAt(message.ts ?? messageTs),
    permalink: slackWebLink(teamId, channelId, messageTs),
  };
}

/** Slack wraps links as `<https://…|label>` or `<https://…>`. */
export function firstLink(text: string): string | null {
  const angled = text.match(/<(https?:\/\/[^|>\s]+)(\|[^>]*)?>/);
  if (angled) return angled[1];
  const bare = text.match(/https?:\/\/[^\s<>]+/);
  return bare ? bare[0] : null;
}

/** The text a person typed after the link in a slash command. */
export function noteAfterLink(text: string): string {
  const link = firstLink(text);
  if (!link) return "";
  return text
    .replace(/<[^>]*>/g, "")
    .replace(link, "")
    .trim();
}

function truncate(text: string, limit = MAX_SLACK_TEXT): string {
  if (text.length <= limit) return text;
  return `${text.slice(0, limit - 1)}…`;
}

/**
 * The message that reports the outcome of an add. It is plain text (with a
 * fenced block for the trace) so it works as an ephemeral reply and as a DM.
 */
export function resultText(result: AddResult): string {
  if (result.ok) {
    const title = result.task?.title ?? "the task";
    const lines = [`:white_check_mark: Added to your TimeSlay inbox: *${title}*`];
    if (result.task?.link && /^https?:\/\//.test(result.task.link)) {
      lines.push(`<${result.task.link}|Open the task>`);
    }
    if (result.via) lines.push(`_via ${result.via}_`);
    return lines.join("\n");
  }
  const error = result.error ?? "TimeSlay could not add this task.";
  const lines = [`:x: ${error}`];
  if (result.trace) lines.push(truncate(`\`\`\`${result.trace}\`\`\``));
  return lines.join("\n");
}

/** The optional dates a task can carry, in the shape the native host expects. */
export type TaskDates = {
  /** `YYYY-MM-DD`. */
  due?: string;
  /** `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`. */
  deadline?: string;
};

/** Adds a task and reports the outcome through `report`. */
async function addAndReport(
  options: AddOptions,
  task: BuiltTask,
  dates: TaskDates,
  report: (result: AddResult) => Promise<unknown>,
): Promise<void> {
  let result: AddResult;
  try {
    result = await addToInbox(
      {
        command: "add-to-inbox",
        url: task.url,
        title: task.title,
        description: task.description,
        source: task.source,
        ...(dates.due ? { due: dates.due } : {}),
        ...(dates.deadline ? { deadline: dates.deadline } : {}),
      },
      options,
    );
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    result = { ok: false, error: message, trace: String(error) };
  }
  await report(result);
}

/** Reads the message a Slack permalink points at, so the task keeps its text. */
async function readLinkedMessage(
  client: WebClient,
  link: { channel: string; ts: string },
): Promise<SlackMessage | null> {
  try {
    const response = await client.conversations.history({
      channel: link.channel,
      latest: link.ts,
      oldest: link.ts,
      inclusive: true,
      limit: 1,
    });
    return (response.messages?.[0] as SlackMessage | undefined) ?? null;
  } catch {
    // The bot may not be in the channel, or the history scope may be absent.
    // The link on its own is still a fine task.
    return null;
  }
}

/** An ephemeral `respond` that shows the outcome in the invoking channel. */
async function reportEphemeral(respond: RespondFn, result: AddResult): Promise<void> {
  await respond({ text: resultText(result), response_type: "ephemeral" });
}

/** A DM report, used where no response URL exists (a form submission). */
async function reportDirect(
  client: WebClient,
  userId: string,
  result: AddResult,
): Promise<void> {
  try {
    await client.chat.postMessage({ channel: userId, text: resultText(result) });
  } catch (error) {
    // A DM may be impossible, but the task is already written; log rather than
    // throw so the user still gets the Slack acknowledgement.
    console.error("Could not DM the result:", error);
  }
}

/**
 * Builds a task from a Slack link when the bot can read the message, or from
 * the link and an optional note/title when it cannot.
 */
export async function taskFromInput(
  client: WebClient,
  url: string,
  note: string,
  typedTitle: string,
  plumbing: Pick<TaskContext, "author" | "channel" | "sentAt"> = {},
): Promise<BuiltTask> {
  const link = parseSlackLink(url);
  const message = link ? await readLinkedMessage(client, link) : null;

  if (message) {
    const context: TaskContext = {
      ...plumbing,
      author: message.user ? `<@${message.user}>` : plumbing.author,
      sentAt: sentAt(message.ts ?? link?.ts),
      permalink: url,
    };
    const body = note ? `${note}\n\n${messageBody(message)}` : messageBody(message);
    const task = buildTask(body, context, typedTitle || undefined);
    // A title the person typed wins over the first line of the message.
    if (typedTitle) task.title = typedTitle;
    return task;
  }

  const context: TaskContext = { ...plumbing, permalink: url };
  const task = buildTask(note, context, typedTitle || taskTitle(note, url));
  if (typedTitle) task.title = typedTitle;
  return task;
}

/** Registers every listener on an app. Split out so it can be tested. */
export function register(app: App, config: Config): void {
  const options = addOptions(config);

  // The main path: "Add to inbox" on a message's actions menu.
  app.shortcut(MESSAGE_SHORTCUT, async ({ shortcut, ack, respond, client, logger }) => {
    await ack();
    if (shortcut.type !== "message_action") return;

    const message = shortcut.message as SlackMessage;
    const context = contextFor(
      message,
      shortcut.channel.id,
      shortcut.message_ts,
      shortcut.team?.id ?? "",
      `#${shortcut.channel.name}`,
    );
    logger.info(
      `Adding Slack message ${shortcut.channel.id}/${shortcut.message_ts} to the inbox`,
    );
    const body = messageBody(message);
    await addAndReport(options, buildTask(body, context), {}, (result) =>
      reportEphemeral(respond, result),
    );
  });

  // A global shortcut has no message, so ask for a link first.
  app.shortcut(GLOBAL_SHORTCUT, async ({ shortcut, ack, client }) => {
    await ack();
    await client.views.open({
      trigger_id: shortcut.trigger_id,
      view: {
        type: "modal",
        callback_id: MODAL_CALLBACK,
        title: { type: "plain_text", text: "Add to TimeSlay" },
        submit: { type: "plain_text", text: "Add to inbox" },
        close: { type: "plain_text", text: "Cancel" },
        blocks: [
          {
            type: "input",
            block_id: "link",
            label: { type: "plain_text", text: "Link" },
            element: {
              type: "plain_text_input",
              action_id: "value",
              placeholder: {
                type: "plain_text",
                text: "Paste a link or a Slack message link",
              },
            },
          },
          {
            type: "input",
            block_id: "title",
            optional: true,
            label: { type: "plain_text", text: "Title (optional)" },
            element: { type: "plain_text_input", action_id: "value" },
          },
          {
            type: "input",
            block_id: "note",
            optional: true,
            label: { type: "plain_text", text: "Note (optional)" },
            element: { type: "plain_text_input", action_id: "value", multiline: true },
          },
          {
            type: "input",
            block_id: "due",
            optional: true,
            label: { type: "plain_text", text: "Due date (optional)" },
            element: {
              type: "datepicker",
              action_id: "value",
              placeholder: { type: "plain_text", text: "Pick a day" },
            },
          },
          {
            type: "input",
            block_id: "deadline",
            optional: true,
            label: { type: "plain_text", text: "Deadline (optional)" },
            element: {
              type: "datepicker",
              action_id: "value",
              placeholder: { type: "plain_text", text: "Must be finished by" },
            },
          },
        ],
      },
    });
  });

  // The form the global shortcut opened.
  app.view(
    { callback_id: MODAL_CALLBACK, type: "view_submission" },
    async ({ ack, body, view, client, logger }) => {
      const values = view.state.values;
      const url = (values.link?.value?.value ?? "").trim();
      const typedTitle = (values.title?.value?.value ?? "").trim();
      const note = (values.note?.value?.value ?? "").trim();
      const due = (values.due?.value?.selected_date ?? "").trim();
      const deadline = (values.deadline?.value?.selected_date ?? "").trim();

      if (!url) {
        await ack({
          response_action: "errors",
          errors: { link: "Enter a link to add." },
        });
        return;
      }
      await ack();

      const userId = body.user.id;
      const task = await taskFromInput(client, url, note, typedTitle);
      logger.info(`Adding ${task.url} to the inbox for ${userId}`);
      await addAndReport(
        options,
        task,
        { ...(due ? { due } : {}), ...(deadline ? { deadline } : {}) },
        (result) => reportDirect(client, userId, result),
      );
    },
  );

  // A slash command: /add-to-inbox <link> [note]
  app.command(COMMAND, async ({ command, ack, respond, client, logger }) => {
    await ack();
    const url = firstLink(command.text);
    if (!url) {
      await respond({
        response_type: "ephemeral",
        text:
          `Usage: \`${COMMAND} <link> [note]\`\n` +
          "Paste a web link or a Slack message link (copy its link from the ⋯ menu).",
      });
      return;
    }
    const note = noteAfterLink(command.text);
    const task = await taskFromInput(client, url, note, "", {
      channel: command.channel_name ? `#${command.channel_name}` : undefined,
    });

    logger.info(`Adding ${task.url} to the inbox for ${command.user_name}`);
    await addAndReport(options, task, {}, (result) => reportEphemeral(respond, result));
  });

  // The App Home tab explains where the action appears.
  app.event("app_home_opened", async ({ event, client }) => {
    try {
      await client.views.publish({
        user_id: event.user,
        view: {
          type: "home",
          blocks: [
            { type: "header", text: { type: "plain_text", text: "TimeSlay" } },
            {
              type: "section",
              text: {
                type: "mrkdwn",
                text: "Add a Slack message to your TimeSlay inbox.",
              },
            },
            { type: "divider" },
            {
              type: "section",
              text: {
                type: "mrkdwn",
                text:
                  "*From a message*\n" +
                  "Hover a message, open the ⋯ menu, and choose *Add to inbox*. " +
                  "Everything in the message and a link that opens it in Slack are saved.",
              },
            },
            {
              type: "section",
              text: {
                type: "mrkdwn",
                text:
                  "*From anywhere*\n" +
                  `Use the global shortcut *Add to inbox*, or run \`${COMMAND} <link> [note]\`.`,
              },
            },
            {
              type: "context",
              elements: [
                {
                  type: "mrkdwn",
                  text: "TimeSlay must be able to run the `timeslay` binary on this machine.",
                },
              ],
            },
          ],
        },
      });
    } catch (error) {
      console.error("Could not publish the App Home view:", error);
    }
  });
}

/** Checks the actor, never the author of a message selected by a shortcut. */
export function actorId(body: unknown): string | undefined {
  const value = body as { user?: string | { id?: string }; user_id?: string; event?: { user?: string } };
  return (typeof value.user === "string" ? value.user : value.user?.id) ??
    value.user_id ?? value.event?.user;
}

export function installOwnerGuard(app: App, distributed: boolean, legacyOwner?: string): void {
  app.use(async (args) => {
    const owner = distributed ? args.context.ownerUserId : legacyOwner;
    if ((!distributed && !owner) || (typeof owner === "string" && actorId(args.body) === owner)) {
      await args.next();
      return;
    }
    if ("ack" in args && typeof args.ack === "function") await args.ack();
    if ("respond" in args && typeof args.respond === "function") {
      await args.respond({ response_type: "ephemeral", text: "This TimeSlay inbox belongs to another Slack account." });
    }
  });
}

/** Builds the Bolt app and attaches every listener. */
export function createApp(config: Config, installations?: Installations): App {
  if (config.oauth && !installations) throw new Error("OAuth mode requires an installation store.");
  const app = new App({
    ...(config.oauth && installations ? {
      authorize: authorizeInstallation(installations),
    } : { token: config.botToken }),
    appToken: config.appToken,
    socketMode: true,
  });
  installOwnerGuard(app, !!config.oauth, config.allowedUserId);
  register(app, config);
  app.error(async (error) => {
    console.error("Unhandled Slack app error:", error);
  });
  return app;
}

const invokedDirectly =
  process.argv[1] !== undefined &&
  import.meta.url === pathToFileURL(process.argv[1]).href;

if (invokedDirectly) {
  const config = loadConfig();
  const tokens = config.appRotation ? new AppTokens(config.appRotation) : undefined;
  while (tokens) {
    try {
      config.appToken = await tokens.getToken();
      break;
    } catch {
      console.error("Could not refresh or save Slack app tokens; check client credentials and token-cache permissions. Retrying in 30 seconds.");
      await delay(30_000);
    }
  }
  const installations = config.oauth ? new Installations(config.oauth.installationsFile, config.oauth.clientId) : undefined;
  if (config.oauth && installations) await startOAuth(config.oauth, installations);
  let app = createApp(config, installations);
  await app.start();
  console.log("TimeSlay Slack app is running (socket mode).");
  let retrying = false;
  while (tokens) {
    if (!retrying) await delay(Math.max(1_000, tokens.refreshAt - Date.now()));
    try {
      // Save the replacement pair before reconnecting Socket Mode. The new
      // receiver also uses this token for all subsequent automatic reconnects.
      const appToken = await tokens.getToken();
      await app.stop();
      config.appToken = appToken;
      app = createApp(config, installations);
      await app.start();
      retrying = false;
      console.log("Slack app-level token rotated; Socket Mode reconnected.");
    } catch {
      retrying = true;
      console.error("Slack token rotation or reconnection failed; retrying in 30 seconds.");
      await delay(30_000);
    }
  }
}

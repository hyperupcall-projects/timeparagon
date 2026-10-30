export type SlackAttachment = {
  title?: string;
  text?: string;
  fallback?: string;
  from_url?: string;
  original_url?: string;
};

export type SlackFile = {
  id?: string;
  name?: string;
  title?: string;
  permalink?: string;
  url_private?: string;
  mimetype?: string;
};

export type SlackMessage = {
  text?: string;
  user?: string;
  ts?: string;
  thread_ts?: string;
  permalink?: string;
  attachments?: SlackAttachment[];
  files?: SlackFile[];
  blocks?: unknown[];
};

export type TaskContext = {
  author?: string;
  channel?: string;
  sentAt?: string;
  permalink: string;
};

export type BuiltTask = {
  url: string;
  title: string;
  description: string;
  source: "slack";
};

/**
 * Everything a person can read in the message, including attachments and
 * files. The text is deliberately not shortened: TimeSlay puts this in the
 * task's description.
 */
export function messageBody(message: SlackMessage): string {
  const parts: string[] = [];
  const text = message.text?.trim();
  if (text) parts.push(text);

  for (const attachment of message.attachments ?? []) {
    const lines = [attachment.title, attachment.text, attachment.fallback]
      .map((value) => value?.trim())
      .filter((value): value is string => Boolean(value));
    if (lines.length > 0) parts.push(`[Attachment]\n${lines.join("\n")}`);
  }

  for (const file of message.files ?? []) {
    const name = file.title?.trim() || file.name?.trim() || file.id?.trim() || "file";
    const link = file.permalink?.trim() || file.url_private?.trim() || "";
    parts.push(link ? `[File] ${name}\n${link}` : `[File] ${name}`);
  }

  if (parts.length === 0 && message.blocks && message.blocks.length > 0) {
    // A blocks-only message (a card, for example) has no plain text. Keep the
    // raw blocks so nothing is silently lost.
    parts.push(JSON.stringify(message.blocks, null, 2));
  }

  return parts.length > 0 ? parts.join("\n\n") : "(this message has no text)";
}

/** A short, single-line task title derived from the message body. */
export function taskTitle(body: string, fallback = "Slack message"): string {
  const first = body
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line.length > 0);
  const cleaned = (first ?? fallback)
    .replace(/<https?:\/\/[^|>]+\|([^>]+)>/g, "$1")
    .replace(/<https?:\/\/[^>]+>/g, "")
    .replace(/<@([A-Z0-9]+)>/g, "@$1")
    .replace(/<#[A-Z0-9]+\|([^>]+)>/g, "#$1")
    .replace(/<!([^>]+)>/g, "@$1")
    .replace(/[*_~`]/g, "")
    .trim();
  const value = cleaned || fallback;
  const characters = Array.from(value);
  return characters.length > 120
    ? `${characters.slice(0, 119).join("")}…`
    : value;
}

/** Slack's timestamp is seconds.microseconds, as a string. */
export function sentAt(ts: string | undefined): string {
  if (!ts) return "";
  const seconds = Number(ts.split(".")[0]);
  if (!Number.isFinite(seconds) || seconds <= 0) return "";
  return new Date(seconds * 1000).toISOString();
}

/**
 * A web link that opens the message in Slack's client. Slack redirects it to
 * the desktop app when that is installed.
 */
export function slackWebLink(teamId: string, channelId: string, ts: string): string {
  return `https://app.slack.com/client/${teamId}/${channelId}/${ts}`;
}

export type SlackLink = {
  channel: string;
  ts: string;
  team?: string;
};

/**
 * Reads the three link shapes Slack hands out: a channel permalink, an
 * app.slack.com client link, and the slack:// deep link.
 */
export function parseSlackLink(value: string): SlackLink | null {
  const url = value.trim();
  const archive = url.match(/\/archives\/([A-Z0-9]+)\/p(\d{10})(\d{6})/);
  if (archive) {
    return { channel: archive[1], ts: `${archive[2]}.${archive[3]}` };
  }

  const client = url.match(
    /\/client\/([A-Z0-9]+)\/([A-Z0-9]+)\/(\d+\.\d+)/,
  );
  if (client) {
    return { team: client[1], channel: client[2], ts: client[3] };
  }

  if (url.startsWith("slack://")) {
    try {
      const parsed = new URL(url);
      const channel = parsed.searchParams.get("id");
      const ts = parsed.searchParams.get("message");
      const team = parsed.searchParams.get("team") ?? undefined;
      if (channel && ts) return { channel, ts, team };
    } catch {
      return null;
    }
  }
  return null;
}

/** The description TimeSlay stores: who, where, when, the link, then the text. */
export function buildTaskDescription(body: string, context: TaskContext): string {
  const lines: string[] = [];
  if (context.author) lines.push(`From: ${context.author}`);
  if (context.channel) lines.push(`Channel: ${context.channel}`);
  if (context.sentAt) lines.push(`Sent: ${context.sentAt}`);
  lines.push(`Link: ${context.permalink}`);
  return `${lines.join("\n")}\n\n${body}`;
}

export function buildTask(
  body: string,
  context: TaskContext,
  fallbackTitle?: string,
): BuiltTask {
  return {
    url: context.permalink,
    title: taskTitle(body, fallbackTitle),
    description: buildTaskDescription(body, context),
    source: "slack",
  };
}

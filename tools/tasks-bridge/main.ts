//! Tasks bridge: reads the *time* of a Google Task out of the browser, because
//! the Google Tasks API only ever returns a date (`due` is midnight UTC).
//!
//! The Rust app already has the date and the task id from the API. This tool's
//! only job is to report the time-of-day the Tasks UI shows for each task, so
//! the two can be joined. That split matters: date comes from the API, time
//! comes from here, and neither side has to guess at the other's timezone.
//!
//! Design choice (reliability over convenience): a real Chrome profile that the
//! user signs into once by hand. We never script a login, never export a
//! portable credential file, and never open a debugging port. See README.md.
//!
//! Commands: signin | status | dump | fetch | selftest

import { chromium } from "npm:playwright@1.63.0";

const KEEP_COMMAND = (Deno.args[0] ?? "").startsWith("keep-");
const TASKS_URL = KEEP_COMMAND
  ? "https://keep.google.com/"
  : Deno.env.get("TASKS_BRIDGE_URL") ?? "https://tasks.google.com/";
const HEADED = Deno.env.get("TASKS_BRIDGE_HEADED") === "1";

// The Tasks UI is locale-formatted. Pinning the locale makes the strings we
// parse deterministic; pinning the timezone to the machine's real zone keeps
// the rendered wall-clock time correct (pinning it to UTC would not).
const LOCAL_TZ = Intl.DateTimeFormat().resolvedOptions().timeZone;

/// The supported way to run Chrome without its sandbox is this launch option:
/// Playwright then passes `--no-sandbox` itself. `--no-sandbox` is not a
/// Playwright, Deno, or `deno compile` flag, so passing it directly anywhere
/// fails as an unsupported flag -- this option is the fix for that.
const NO_SANDBOX = Deno.env.get("TASKS_BRIDGE_NO_SANDBOX") === "1";

function dataDir(): string {
  const xdg = Deno.env.get("XDG_DATA_HOME");
  const home = Deno.env.get("HOME") ?? ".";
  return xdg
    ? `${xdg}/timetracker/tasks-bridge`
    : `${home}/.local/share/timetracker/tasks-bridge`;
}

/// Kept out of the repo on purpose: the profile contains live Google session
/// cookies and is a credential in its own right.
function profileDir(): string {
  return Deno.env.get("TASKS_BRIDGE_PROFILE") ?? `${dataDir()}/chrome-profile`;
}

function dumpDir(): string {
  return Deno.env.get("TASKS_BRIDGE_DUMPS") ?? `${dataDir()}/dumps`;
}

async function launch(headless: boolean) {
  const dir = profileDir();
  await Deno.mkdir(dir, { recursive: true });
  return chromium.launchPersistentContext(dir, {
    headless,
    // Use the installed Chrome rather than a bundled download. Fewer moving
    // parts, a real fingerprint, and nothing to ship alongside the binary.
    channel: "chrome",
    locale: "en-US",
    timezoneId: Deno.env.get("TASKS_BRIDGE_TIMEZONE") ?? LOCAL_TZ,
    viewport: { width: 1280, height: 900 },
    ...(NO_SANDBOX ? { chromiumSandbox: false } : {}),
  });
}

type Ctx = Awaited<ReturnType<typeof launch>>;

function primaryPage(ctx: Ctx) {
  return ctx.pages()[0] ?? ctx.newPage();
}

type Auth = "signed-in" | "signed-out" | "unknown";

/// Auth is decided by DOM evidence, never by URL or title.
///
/// A signed-out visit to tasks.google.com still serves a document *titled*
/// "Google Tasks"; only the body is the Google sign-in form. So neither the URL
/// nor the title distinguishes the two states. The reliable giveaway is the
/// sign-in surface, the `#identifierId` email field, which `dump` confirmed on
/// the signed-out page.
async function authState(ctx: Ctx): Promise<Auth> {
  let onAppHost = false;
  for (const p of ctx.pages()) {
    const url = p.url();
    if (url === "" || url.startsWith("about:") || url.startsWith("chrome")) {
      continue;
    }
    if (url.includes("accounts.google.com")) return "signed-out";
    const signIn = await p
      .locator('#identifierId, input[name="identifier"]')
      .count()
      .catch(() => 0);
    if (signIn > 0) return "signed-out";
    if (
      url.startsWith("https://tasks.google.com") ||
      (KEEP_COMMAND && url.startsWith("https://keep.google.com"))
    ) onAppHost = true;
  }
  return onAppHost ? "signed-in" : "unknown";
}

/// The app hydrates after a redirect, so poll until the state is definite
/// rather than trusting a single reading.
async function waitForAuth(ctx: Ctx, timeoutMs: number): Promise<Auth> {
  const deadline = Date.now() + timeoutMs;
  let last: Auth = "unknown";
  while (Date.now() < deadline) {
    last = await authState(ctx);
    if (last !== "unknown") return last;
    await new Promise((r) => setTimeout(r, 500));
  }
  return last;
}

const ROW_SELECTOR = Deno.env.get("TASKS_BRIDGE_ROW_SELECTOR") ??
  '[role="listitem"]';

type Parsed = { hhmm: string; ambiguous: boolean };

/// Parse a display string like "9:30 AM". When there is no meridiem and the
/// hour could be 12- or 24-hour, we report it as ambiguous rather than pick
/// one -- a wrong time silently placed on the day grid is worse than a task
/// that stays all-day.
function parseDisplayTime(raw: string | null | undefined): Parsed | null {
  if (!raw) return null;
  const m = /\b(\d{1,2}):(\d{2})\s*(a\.?m\.?|p\.?m\.?)?/i.exec(raw);
  if (!m) return null;
  let h = Number(m[1]);
  const min = Number(m[2]);
  const mer = m[3]?.toLowerCase().replace(/\./g, "") ?? null;
  if (h > 23 || min > 59) return null;
  if (mer === "pm" && h < 12) h += 12;
  if (mer === "am" && h === 12) h = 0;
  const hhmm = `${String(h).padStart(2, "0")}:${String(min).padStart(2, "0")}`;
  return { hhmm, ambiguous: mer === null && Number(m[1]) <= 12 };
}

const SCHEDULE_PREFIX = "Scheduled for ";

/// One task row, read straight out of the DOM. The shape came from `dump`,
/// not from guessing:
///   - rows are `[role="listitem"]` inside `[role="list"]`;
///   - the schedule is a child whose aria-label is
///     `Scheduled for <date>[, <time>]`;
///   - the title is a child with `data-aria-label="Task title"`, its text in
///     `data-rendered-value`;
///   - the list name is the nearest heading preceding the containing list.
///
/// None of this is the public API's task id: the web app's `data-task-id` is a
/// 16-char internal id, so the join with the API is by title, not id.
type Row = {
  list: string | null;
  section: string | null;
  taskId: string | null;
  title: string | null;
  schedule: string | null;
};

/// Read every task row in a single round trip.
async function readRows(
  page: import("npm:playwright@1.63.0").Page,
  selector: string,
): Promise<Row[]> {
  return await page.evaluate((sel: string) => {
    const lists = Array.from(document.querySelectorAll('[role="list"]'));
    const headings = Array.from(document.querySelectorAll("h1,h2,h3"));
    const nameOfList = new Map<Element, string>();
    for (const l of lists) {
      const before = headings.filter((h) =>
        l.compareDocumentPosition(h) & Node.DOCUMENT_POSITION_PRECEDING
      );
      const h = before[before.length - 1];
      nameOfList.set(l, h ? (h.textContent ?? "").trim() : "");
    }

    const out: Row[] = [];
    for (const row of Array.from(document.querySelectorAll(sel))) {
      const listEl = row.closest('[role="list"]');
      const sched = row.querySelector('[aria-label^="Scheduled for"]');
      const titleEl = row.querySelector('[data-aria-label="Task title"]');
      const idEl = row.matches("[data-task-id]")
        ? row
        : row.querySelector("[data-task-id]");
      const rendered = titleEl?.getAttribute("data-rendered-value") ?? "";
      out.push({
        list: listEl ? nameOfList.get(listEl) ?? null : null,
        section: listEl?.getAttribute("aria-label") ?? null,
        taskId: idEl?.getAttribute("data-task-id") ?? null,
        title: rendered || (titleEl?.textContent ?? "").trim() || null,
        schedule: sched?.getAttribute("aria-label") ?? null,
      });
    }
    return out;
  }, selector);
}

/// Split `Scheduled for Today, 7:00 PM` into a date phrase and a 24-hour time.
///
/// Date-only labels -- "Scheduled for 2 days ago", "Scheduled for Monday,
/// October 5, 2026" -- yield a null time on purpose: those tasks carry no time
/// and must stay all-day rather than be placed at a guessed hour.
function splitSchedule(raw: string | null): {
  dateText: string | null;
  time: string | null;
  ambiguous: boolean;
} {
  if (!raw) return { dateText: null, time: null, ambiguous: false };
  const body = raw.startsWith(SCHEDULE_PREFIX)
    ? raw.slice(SCHEDULE_PREFIX.length)
    : raw;
  const parsed = parseDisplayTime(body);
  const dateText =
    body.replace(/\s*,?\s*\d{1,2}:\d{2}\s*(a\.?m\.?|p\.?m\.?)?/i, "").trim() ||
    null;
  return {
    dateText,
    time: parsed?.hhmm ?? null,
    ambiguous: parsed?.ambiguous ?? false,
  };
}

async function cmdSignin(): Promise<void> {
  const dir = profileDir();
  await Deno.mkdir(dir, { recursive: true });

  // Sign in with a *plain* Chrome -- one Playwright is not driving.
  //
  // This is the fix for "Couldn't sign you in. This browser or app may not be
  // secure." Google refuses sign-in when the browser is under automation, and
  // Playwright's default args set `navigator.webdriver`. A normal Chrome window
  // driven by a human is not blocked. Playwright only takes over afterwards, on
  // a session that is already authenticated.
  const chrome = Deno.env.get("TASKS_BRIDGE_CHROME") ?? "google-chrome";
  console.error(
    `Opening Chrome with a dedicated profile. Sign in, wait for Google ${
      KEEP_COMMAND ? "Keep" : "Tasks"
    }\n` +
      `to load, then close that window.\n` +
      `  profile: ${dir}\n` +
      `  chrome:  ${chrome}`,
  );

  const child = new Deno.Command(chrome, {
    args: [
      `--user-data-dir=${dir}`,
      // Match Playwright's cookie encryption *exactly*.
      //
      // Playwright launches Chromium with `--password-store=basic` (it is in
      // its default args), which encrypts cookies with a hardcoded key. A
      // normal desktop Chrome instead derives the key from the OS keyring
      // (gnome-keyring / kwallet). The two keys are mutually unreadable, so a
      // session written by one is invisible to the other -- the sign-in looks
      // successful, then Playwright sees an empty cookie jar and reports
      // "signed-out". Using the same store on both sides is what makes the
      // handoff work.
      "--password-store=basic",
      "--use-mock-keychain",
      "--no-first-run",
      "--no-default-browser-check",
      TASKS_URL,
    ],
    stdin: "null",
    stdout: "null",
    stderr: "null",
  }).spawn();

  const { code } = await child.status;
  if (code !== 0) console.error(`Chrome exited with code ${code}.`);

  // Verify with Playwright that the profile now carries a real session.
  const ctx = await launch(true);
  const page = await primaryPage(ctx);
  await page.goto(TASKS_URL, { waitUntil: "domcontentloaded" });
  const state = await waitForAuth(ctx, 20_000);
  if (KEEP_COMMAND) {
    try {
      const { waitForKeep } = await import("./keep.ts");
      await waitForKeep(page);
    } finally {
      await ctx.close();
    }
  } else {
    await ctx.close();
  }

  if (state !== "signed-in") {
    console.error(`Still not signed in (${state}).`);
    Deno.exit(2);
  }
  console.log(JSON.stringify({ signedIn: true, profile: dir }));
}

async function cmdStatus(): Promise<void> {
  const ctx = await launch(true);
  const page = await primaryPage(ctx);
  await page.goto(TASKS_URL, { waitUntil: "domcontentloaded" });
  const state = await waitForAuth(ctx, 20_000);
  const url = page.url();
  const cookies = await ctx.cookies();
  const google = cookies.filter((c) => c.domain.includes("google"));
  await ctx.close();
  const signedIn = state === "signed-in";
  // The cookie counts tell apart two situations that look identical from the
  // page alone: no session at all, versus a session that is present in the
  // profile but that Chrome could not decrypt (a password-store mismatch).
  console.log(JSON.stringify({
    signedIn,
    state,
    url,
    cookies: cookies.length,
    googleCookies: google.length,
  }));
  // Distinct exit code so the app can raise "Reconnect" rather than showing
  // an empty day.
  if (!signedIn) Deno.exit(3);
}

async function cmdDump(): Promise<void> {
  const ctx = await launch(true);
  const page = await primaryPage(ctx);
  await page.goto(TASKS_URL, { waitUntil: "domcontentloaded" });
  const state = await waitForAuth(ctx, 20_000);
  if (state !== "signed-in") {
    await ctx.close();
    console.error(`Not signed in (${state}). Run \`deno task signin\` first.`);
    Deno.exit(3);
  }
  await new Promise((r) => setTimeout(r, 3000));

  const dir = dumpDir();
  await Deno.mkdir(dir, { recursive: true });

  // Accessibility tree first: it names roles/labels without obfuscated classes.
  await Deno.writeTextFile(
    `${dir}/aria.txt`,
    await page.locator("body").ariaSnapshot(),
  );
  await Deno.writeTextFile(`${dir}/page.html`, await page.content());

  // Everything that looks like a time or an id, so the real selector is found
  // by evidence rather than guessed.
  const candidates = await page.evaluate(() => {
    const TIME = /(\d{1,2}:\d{2}\s*(?:[ap]\.?m\.?)?)/i;
    const out: unknown[] = [];
    for (const el of Array.from(document.querySelectorAll("*"))) {
      const attrs: Record<string, string> = {};
      for (const a of Array.from(el.attributes)) attrs[a.name] = a.value;
      const attrBlob = Object.entries(attrs).map(([k, v]) => `${k}=${v}`).join(
        " ",
      );
      const text = (el.textContent ?? "").trim();
      const hit = TIME.exec(text)?.[1] ?? TIME.exec(attrBlob)?.[1] ?? null;
      const idLike = Object.keys(attrs).some((k) =>
        /id|task|time|date|due|start/i.test(k)
      );
      if ((hit || idLike) && el.children.length <= 8) {
        out.push({
          tag: el.tagName.toLowerCase(),
          attrs,
          text: text.slice(0, 200),
          time: hit,
        });
      }
      if (out.length > 4000) break;
    }
    return out;
  });

  await Deno.writeTextFile(
    `${dir}/candidates.json`,
    JSON.stringify(candidates, null, 2),
  );
  await ctx.close();

  console.log(
    JSON.stringify({ wrote: dir, candidates: candidates.length }, null, 2),
  );
}

async function cmdFetch(): Promise<void> {
  const ctx = await launch(!HEADED);
  const page = await primaryPage(ctx);
  await page.goto(TASKS_URL, { waitUntil: "domcontentloaded" });
  const state = await waitForAuth(ctx, 20_000);
  if (state !== "signed-in") {
    await ctx.close();
    console.error(`Not signed in (${state}). Run \`deno task signin\` first.`);
    Deno.exit(3);
  }
  await new Promise((r) => setTimeout(r, 4000));

  const rows = await readRows(page, ROW_SELECTOR);
  await ctx.close();

  const tasks = rows.map((row) => {
    const s = splitSchedule(row.schedule);
    return {
      ...row,
      dateText: s.dateText,
      time: s.time,
      ambiguous: s.ambiguous,
    };
  });

  // A label that looks like it holds a time but did not parse is the only real
  // anomaly worth surfacing. A date-only or unscheduled task is normal.
  const warnings = tasks
    .filter((t) => t.schedule && /\d:\d{2}/.test(t.schedule) && !t.time)
    .map((t) => `unparsed schedule: ${t.schedule} (${t.title ?? t.taskId})`);

  // Time-of-day only: the app already has the date from the API. JSON on
  // stdout, everything else on stderr, so this stays pipeable.
  console.log(JSON.stringify(
    {
      selector: ROW_SELECTOR,
      count: tasks.length,
      withTime: tasks.filter((t) => t.time).length,
      tasks,
      warnings,
    },
    null,
    2,
  ));
}

async function cmdSelftest(): Promise<void> {
  const ctx = await launch(true);
  const page = await primaryPage(ctx);
  await page.goto("about:blank");
  const version = ctx.browser()?.version() ?? "unknown";
  // These two are the things Google's sign-in looks at.
  const probe = await page.evaluate(() => ({
    webdriver: navigator.webdriver,
    ua: navigator.userAgent,
  }));
  await ctx.close();
  console.log(JSON.stringify({
    ok: true,
    chrome: version,
    timezone: LOCAL_TZ,
    noSandbox: NO_SANDBOX,
    ...probe,
  }));
}

// Serialize every Tasks/Keep process sharing this profile (also across app
// instances). OS advisory locks release automatically if a process crashes.
await Deno.mkdir(profileDir(), { recursive: true, mode: 0o700 });
const lock = await Deno.open(`${profileDir()}.bridge-lock`, {
  create: true,
  write: true,
  mode: 0o600,
});
await lock.lock(true);
const command = Deno.args[0] ?? "selftest";
try {
  switch (command) {
    case "keep-fetch": {
      const ctx = await launch(true);
      try {
        const { fetchKeep } = await import("./keep.ts");
        console.log(JSON.stringify(await fetchKeep(ctx)));
      } finally {
        await ctx.close();
      }
      break;
    }
    case "keep-signin":
    case "signin":
      await cmdSignin();
      break;
    case "status":
      await cmdStatus();
      break;
    case "dump":
      await cmdDump();
      break;
    case "fetch":
      await cmdFetch();
      break;
    case "selftest":
      await cmdSelftest();
      break;
    default:
      console.error(`Unknown command: ${command}`);
      console.error(
        "Commands: signin | status | dump | fetch | selftest | keep-fetch | keep-signin",
      );
      Deno.exit(2);
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  Deno.exitCode = 2;
} finally {
  lock.close();
}

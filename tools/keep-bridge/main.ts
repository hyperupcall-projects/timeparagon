//! Google Keep browser helper. Task scheduling uses the public APIs only.
import { chromium } from "npm:playwright@1.63.0";
const KEEP_URL = "https://keep.google.com/";
const LOCAL_TZ = Intl.DateTimeFormat().resolvedOptions().timeZone;
// Legacy environment names remain aliases for existing installations.
function setting(name: string): string | undefined {
  return Deno.env.get(`KEEP_BRIDGE_${name}`) ?? Deno.env.get(`TASKS_BRIDGE_${name}`);
}
const NO_SANDBOX = setting("NO_SANDBOX") === "1";
function profileDir(): string {
  const override = setting("PROFILE");
  if (override) return override;
  const xdg = Deno.env.get("XDG_DATA_HOME") ?? `${Deno.env.get("HOME") ?? "."}/.local/share`;
  const current = `${xdg}/time-slay/keep-bridge/chrome-profile`;
  const legacy = `${xdg}/time-slay/tasks-bridge/chrome-profile`;
  // Reuse the old profile in place, including its advisory lock, so renaming
  // the executable does not discard the user's Keep sign-in session.
  for (const profile of [current, legacy]) {
    try {
      if (Deno.statSync(profile).isDirectory) return profile;
    } catch (error) {
      if (!(error instanceof Deno.errors.NotFound)) throw error;
    }
  }
  return current;
}
async function launch(headless: boolean) {
  await Deno.mkdir(profileDir(), { recursive: true });
  return chromium.launchPersistentContext(profileDir(), {
    headless, channel: "chrome", locale: "en-US",
    timezoneId: setting("TIMEZONE") ?? LOCAL_TZ,
    viewport: { width: 1280, height: 900 },
    ...(NO_SANDBOX ? { chromiumSandbox: false } : {}),
  });
}
type Ctx = Awaited<ReturnType<typeof launch>>;
function primaryPage(ctx: Ctx) {
  return ctx.pages()[0] ?? ctx.newPage();
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
  const chrome = setting("CHROME") ?? "google-chrome";
  console.error(
    `Opening Chrome with a dedicated profile. Sign in, wait for Google Keep\n` +
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
      KEEP_URL,
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
  await page.goto(KEEP_URL, { waitUntil: "domcontentloaded" });
  try {
    const { waitForKeep } = await import("./keep.ts");
    await waitForKeep(page);
  } finally {
    await ctx.close();
  }
  console.log(JSON.stringify({ signedIn: true, profile: dir }));
}

// Keep operations sharing the existing profile remain serialized.
await Deno.mkdir(profileDir(), { recursive: true, mode: 0o700 });
const lock = await Deno.open(`${profileDir()}.bridge-lock`, { create: true, write: true, mode: 0o600 });
await lock.lock(true);
try {
  switch (Deno.args[0]) {
    case "keep-fetch": {
      const ctx = await launch(true);
      try {
        const { fetchKeep } = await import("./keep.ts");
        console.log(JSON.stringify(await fetchKeep(ctx)));
      } finally { await ctx.close(); }
      break;
    }
    case "keep-signin": await cmdSignin(); break;
    default: throw new Error("Unknown command. Commands: keep-fetch | keep-signin");
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  Deno.exitCode = 2;
} finally { lock.close(); }

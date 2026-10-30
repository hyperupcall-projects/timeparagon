/* global browser, chrome */
"use strict";

const HOST = "com.timeslay.webextension";
const api = globalThis.browser ?? globalThis.chrome;

const $ = (id) => document.getElementById(id);
let page = { url: "", title: "" };
let lastDebug = "";

/* Talking to the native host ---------------------------------------------- */

function sendNative(message) {
  return new Promise((resolve, reject) => {
    try {
      // Firefox returns a promise; Chrome's callback form is used when the
      // `browser` namespace is absent.
      if (globalThis.browser?.runtime?.sendNativeMessage) {
        globalThis.browser.runtime.sendNativeMessage(HOST, message).then(resolve, reject);
        return;
      }
      globalThis.chrome.runtime.sendNativeMessage(HOST, message, (response) => {
        const error = globalThis.chrome.runtime.lastError;
        if (error) {
          reject(new Error(error.message));
        } else {
          resolve(response);
        }
      });
    } catch (error) {
      reject(error);
    }
  });
}

/* The theme --------------------------------------------------------------- */

/**
 * Paints the popup with the app's chosen theme. The host answers a "theme"
 * request with a flat set of hex colors; each becomes one CSS variable, so the
 * whole stylesheet follows without knowing anything about themes.
 */
function applyTheme(theme) {
  if (!theme) return;
  const root = document.documentElement;
  const colors = {
    "--ts-background": theme.background,
    "--ts-surface": theme.surface,
    "--ts-surface-strong": theme.surface_strong,
    "--ts-text": theme.text,
    "--ts-text-muted": theme.text_muted,
    "--ts-primary": theme.primary,
    "--ts-primary-strong": theme.primary_strong,
    "--ts-on-primary": theme.on_primary,
    "--ts-success": theme.success,
    "--ts-warning": theme.warning,
    "--ts-danger": theme.danger,
  };
  for (const [name, value] of Object.entries(colors)) {
    if (value) root.style.setProperty(name, value);
  }
  root.style.colorScheme = theme.is_dark ? "dark" : "light";
  if (theme.name) $("theme-name").textContent = theme.name;
}

async function loadTheme() {
  try {
    const reply = await sendNative({ command: "theme" });
    if (reply && reply.theme) applyTheme(reply.theme);
  } catch {
    // No host, or an old host without the theme command. The stylesheet's
    // fallback colors stand in, so the popup is still usable.
  }
}

/* The active page --------------------------------------------------------- */

async function readActiveTab() {
  try {
    const tabs = await api.tabs.query({ active: true, currentWindow: true });
    const tab = tabs && tabs[0];
    if (!tab) throw new Error("No active tab was reported by the browser.");
    page = { url: tab.url || "", title: tab.title || "" };
    $("page-title").textContent = page.title || "(untitled page)";
    $("page-url").textContent = page.url || "(this page has no URL)";
    if (!$("title").value) $("title").value = page.title;
    if (!page.url) {
      setStatus(
        "warn",
        "This page has no URL",
        "Open a regular web page (http or https) and try again.",
      );
    }
  } catch (error) {
    showError(error.message || String(error), error.stack || "");
  }
}

/* Status ------------------------------------------------------------------ */

function setStatus(kind, title, detail, trace, link) {
  const status = $("status");
  status.hidden = false;
  status.className = `status ${kind}`;
  $("status-title").textContent = title;

  const detailNode = $("status-detail");
  detailNode.textContent = "";
  detailNode.append(document.createTextNode(detail || ""));
  if (link) {
    detailNode.append(document.createTextNode(" "));
    const anchor = document.createElement("a");
    anchor.href = link;
    anchor.target = "_blank";
    anchor.rel = "noreferrer";
    anchor.textContent = "Open the task";
    detailNode.append(anchor);
  }

  const traceNode = $("trace");
  traceNode.hidden = !trace;
  traceNode.textContent = trace || "";
  $("copy").hidden = !trace;
}

function showError(message, trace) {
  lastDebug = [
    "TimeSlay extension error",
    `host: ${HOST}`,
    `page: ${page.url}`,
    `title: ${page.title}`,
    `message: ${message}`,
    trace ? `\n--- debug trace ---\n${trace}` : "",
  ]
    .filter(Boolean)
    .join("\n");
  setStatus("error", "Could not add to inbox", message, lastDebug);
}

/* Adding ------------------------------------------------------------------ */

/** The form's values, in the shape the native host expects. */
function readForm() {
  const description = $("description").value.trim();
  const deadlineDate = $("deadline-date").value;
  const deadlineTime = $("deadline-time").value;
  const deadline = deadlineDate
    ? deadlineTime
      ? `${deadlineDate} ${deadlineTime}`
      : deadlineDate
    : "";

  return {
    command: "add-to-inbox",
    url: page.url,
    title: $("title").value.trim(),
    description,
    due: $("due").value,
    deadline,
    source: "webextension",
  };
}

async function addToInbox(event) {
  if (event) event.preventDefault();
  const button = $("add");
  button.disabled = true;
  lastDebug = "";
  setStatus("busy", "Adding…", "Reading the link and writing it to your inbox.");

  try {
    const response = await sendNative(readForm());
    if (!response) {
      throw new Error(
        "The native host closed without a reply. Check that the host manifest is installed and that the timeslay binary is built.",
      );
    }
    if (response.ok) {
      const task = response.task || {};
      setStatus(
        "ok",
        "Success!",
        task.title ? `Added “${task.title}” to your inbox.` : "Added to your inbox.",
        null,
        task.link || null,
      );
    } else {
      showError(
        response.error || "The native host reported an error.",
        response.trace || "",
      );
    }
  } catch (error) {
    showError(
      error && error.message ? error.message : String(error),
      error && error.stack ? error.stack : "",
    );
  } finally {
    button.disabled = false;
  }
}

/* Wiring ------------------------------------------------------------------ */

$("form").addEventListener("submit", addToInbox);
$("copy").addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(lastDebug);
    $("copy").textContent = "Copied";
    setTimeout(() => {
      $("copy").textContent = "Copy debug info";
    }, 1200);
  } catch (error) {
    showError("Could not copy the debug info", String(error));
  }
});

loadTheme();
readActiveTab();

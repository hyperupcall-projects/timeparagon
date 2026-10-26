//! Keep is read through its live UI, using the same Chrome launcher/profile as Tasks.
import type { BrowserContext, Page } from "npm:playwright@1.63.0";

const CARD = ".IZ65Hb-n0tgWb.IZ65Hb-WsjYwc-nUpftc";
const CONTENT = ".IZ65Hb-s2gQvd";
const TITLE = ".IZ65Hb-r4nke-haAclf [role=textbox]";

export async function fetchKeep(ctx: BrowserContext) {
  const page = ctx.pages()[0] ?? await ctx.newPage();
  const notes: Record<string, unknown>[] = [];
  const seen = new Set<string>();
  const warnings: string[] = [];
  for (const archived of [false, true]) {
    await page.goto(
      `https://keep.google.com/#${archived ? "archive" : "home"}`,
      { waitUntil: "domcontentloaded" },
    );
    await waitForKeep(page);
    // Keep lazy-loads cards. Scroll to a stable bottom before enumerating them.
    let stable = 0;
    let previous = -1;
    for (let pass = 0; pass < 300; pass++) {
      const height = await page.evaluate(() => {
        const scrollers = [...document.querySelectorAll<HTMLElement>("body *")]
          .filter((e) =>
            e.scrollHeight > e.clientHeight + 100 &&
            /auto|scroll/.test(getComputedStyle(e).overflowY)
          );
        for (const e of scrollers) e.scrollTop = e.scrollHeight;
        window.scrollTo(0, document.body.scrollHeight);
        return Math.max(
          document.body.scrollHeight,
          ...scrollers.map((e) => e.scrollHeight),
        );
      });
      await page.waitForTimeout(300);
      const count = await page.locator(CARD).count();
      const marker = height + count;
      stable = marker === previous ? stable + 1 : 0;
      previous = marker;
      if (stable >= 5) break;
      if (pass === 299) {
        throw new Error(
          "Keep did not finish loading all notes. Retry Refresh.",
        );
      }
    }
    const count = await page.locator(CARD).count();
    console.error(
      `Reading ${count} ${archived ? "archived" : "active"} Keep notes.`,
    );
    for (let index = 0; index < count; index++) {
      const card = page.locator(CARD).nth(index);
      // Keep leaves some previously shared archived cards behind an overlay;
      // those cannot open even in its own UI. Preserve them and explain the
      // limitation rather than dropping them or failing every other note.
      const blocked = await card.locator(".PoC6nf-IZ65Hb-bF1uUb").count() > 0;
      let url: string;
      let id: string;
      if (blocked) {
        url = `https://keep.google.com/#${archived ? "archive" : "home"}`;
        id = `unavailable-${archived}-${index}`;
        const warning =
          "Previously shared notes that Keep will not open remain available. Their Edit button opens the containing Keep page.";
        if (!warnings.includes(warning)) warnings.push(warning);
      } else {
        await card.locator(TITLE).click();
        await page.waitForFunction(
          () => /^#(?:NOTE|LIST)\//.test(location.hash),
          undefined,
          { timeout: 5000 },
        )
          .catch(() => {
            throw new Error(
              `Could not open Keep note ${index + 1} (${
                archived ? "archive" : "active"
              }).`,
            );
          });
        url = page.url();
        id = url.split(/#(?:NOTE|LIST)\//)[1];
        if (!id) throw new Error("Keep did not expose a note link.");
      }
      if (!seen.has(id)) {
        const content = blocked
          ? card.locator(".IZ65Hb-TBnied")
          : page.locator(CONTENT).last();
        await content.waitFor({ state: "visible" });
        // The edited footer's hover tooltip exposes creation and full edit
        // timestamps which Keep abbreviates on the card itself.
        let timestampMetadata: string[] = [];
        const footer = content.locator(".IZ65Hb-jfdpUb-fmcmS").last();
        if (!blocked && await footer.isVisible()) {
          try {
            await footer.hover({ timeout: 1000 });
            const tooltip = page.locator('[role="tooltip"]:visible');
            await tooltip.first().waitFor({ state: "visible", timeout: 1500 });
            timestampMetadata = await tooltip.evaluateAll((elements) =>
              elements.map((e) => (e as HTMLElement).innerText)
            );
          } catch {
            // A tooltip is optional; visible footer metadata is still retained.
          }
        }
        const note = await content.evaluate((element, timestampMetadata) => {
          const text = (selector: string) =>
            (element.querySelector(selector) as HTMLElement | null)
              ?.innerText ?? "";
          const labels = [
            ...element.querySelectorAll(".XPtOyb-di8rgd-Bz112c label"),
          ].map((e) => e.textContent ?? "");
          const metadata = [
            ...element.querySelectorAll(
              ".IZ65Hb-jfdpUb [aria-label], .IZ65Hb-jfdpUb [data-tooltip-text], .IZ65Hb-jfdpUb-fmcmS, .IZ65Hb-hYUzqc [aria-label], time",
            ),
          ]
            .filter((e) =>
              getComputedStyle(e).display !== "none" &&
              !e.closest('[style*="display: none"]')
            )
            .flatMap(
              (e) => [
                e.getAttribute("aria-label"),
                e.getAttribute("data-tooltip-text"),
                e.getAttribute("title"),
                (e as HTMLElement).innerText,
              ],
            )
            .filter((s): s is string => !!s?.trim());
          const body: {
            text: string;
            bold: boolean;
            italic: boolean;
            underline: boolean;
            strikethrough: boolean;
          }[] = [];
          const append = (value: string, parent: Element) => {
            if (!value) return;
            const style = getComputedStyle(parent);
            body.push({
              text: value,
              bold: parseInt(style.fontWeight) >= 600 ||
                style.fontWeight === "bold",
              italic: style.fontStyle === "italic",
              underline: style.textDecorationLine.includes("underline") ||
                !!parent.closest("u"),
              strikethrough:
                style.textDecorationLine.includes("line-through") ||
                !!parent.closest("s, strike, del"),
            });
          };
          const newline = (parent: Element) => {
            if (body.length && !body[body.length - 1].text.endsWith("\n")) {
              append("\n", parent);
            }
          };
          const visit = (node: Node) => {
            if (node.nodeType === Node.TEXT_NODE) {
              if (node.parentElement) {
                append(node.textContent ?? "", node.parentElement);
              }
            } else if (node instanceof HTMLElement) {
              if (getComputedStyle(node).display === "none") return;
              if (node.tagName === "BR") {
                append("\n", node);
                return;
              }
              const block = ["DIV", "P", "LI"].includes(node.tagName);
              if (block) newline(node);
              for (const child of node.childNodes) visit(child);
              if (block) newline(node);
            }
          };
          const description = element.querySelector(".IZ65Hb-qJTHM-haAclf");
          if (description) {
            for (const child of description.childNodes) visit(child);
          }
          if (body.length) {
            body[body.length - 1].text = body[body.length - 1].text.replace(
              /\n$/,
              "",
            );
          }
          return {
            body,
            title: text(".IZ65Hb-r4nke-haAclf [role=textbox]"),
            description: text(".IZ65Hb-qJTHM-haAclf"),
            labels,
            color:
              getComputedStyle(element.closest(".IZ65Hb-TBnied") ?? element)
                .backgroundColor,
            checklist: [...element.querySelectorAll('[role="checkbox"]')].map(
              (e) => {
                const item = e.closest(".rymPhb-ibnC6b-bVEB4e-sM5MNb");
                return {
                  text: item?.querySelector(".rymPhb-ibnC6b-bVEB4e-ij8cu")
                    ?.textContent ?? "",
                  checked: e.getAttribute("aria-checked") === "true",
                  level: item &&
                      parseFloat((item as HTMLElement).style.marginLeft) > 0
                    ? 1
                    : 0,
                };
              },
            ),
            pinned: element.querySelector(
              '[aria-label="Unpin"], [aria-label="Pin"][aria-pressed="true"]',
            ) !== null,
            metadata: [
              ...new Set(
                [...timestampMetadata, ...metadata].filter((s) =>
                  !["Remove label", "Delete task", "Remove source"].includes(s)
                ),
              ),
            ],
          };
        }, timestampMetadata);
        // The original rendered note preserves rich formatting, background art,
        // media, checklist nesting, reminder and collaborator chips exactly.
        // Return preview bytes so the app can display and cache the full snapshot.
        const png = await content.screenshot({
          type: "png",
          animations: "disabled",
          timeout: 15000,
        });
        let binary = "";
        for (const byte of png) binary += String.fromCharCode(byte);
        notes.push({ ...note, id, url, archived, preview: btoa(binary) });
        seen.add(id);
      }
      if (!blocked) {
        await page.keyboard.press("Escape");
        await page.waitForFunction(
          () => !/^#(?:NOTE|LIST)\//.test(location.hash),
          undefined,
          { timeout: 5000 },
        ).catch(() => {
          throw new Error(`Could not close Keep note ${index + 1}.`);
        });
      }
    }
  }
  return { notes, warnings };
}

export async function waitForKeep(page: Page) {
  await page.waitForFunction(
    () => {
      return location.hostname === "accounts.google.com" ||
        !!document.querySelector(
          '#identifierId, input[name="identifier"], [aria-label="Take a note…"], .IZ65Hb-n0tgWb',
        ) || document.body.innerText.includes("Take a note");
    },
    undefined,
    { timeout: 20000 },
  );
  if (
    page.url().includes("accounts.google.com") ||
    await page.locator('#identifierId, input[name="identifier"]').count()
  ) {
    throw new Error(
      "Google Keep is signed out. Use Reconnect and sign in, then close the Chrome window.",
    );
  }
}

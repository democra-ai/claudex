// Reproduce the dead-toolbar-button bug and verify the fix WITHOUT the Tauri
// shell: inject the exact overlay tauri-plugin-decorum creates
// (div[data-tauri-decorum-tb], fixed, top:0, 100%×32px, z-index:100) on top of
// the real built UI, then confirm a toolbar button under it is clickable.
//   - control: force the overlay's pointer-events back to "auto" → click is
//     swallowed, the Backups panel must NOT open.
//   - fixed:   leave our index.css rule (pointer-events:none) → click passes
//     through, the Backups panel MUST open.
import { chromium } from "playwright";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.resolve(HERE, "../../frontend/dist");
const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".json": "application/json" };
function serve() {
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      let p = decodeURIComponent(req.url.split("?")[0]);
      if (p === "/") p = "/index.html";
      let file = path.join(DIST, p);
      if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) file = path.join(DIST, "index.html");
      res.writeHead(200, { "Content-Type": MIME[path.extname(file)] || "application/octet-stream" });
      fs.createReadStream(file).pipe(res);
    });
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

const start = async () => {
  const server = await serve();
  const { port } = server.address();
  const browser = await chromium.launch({ channel: "chrome" });
  const ctx = await browser.newContext({ viewport: { width: 1480, height: 940 }, deviceScaleFactor: 2 });

  await ctx.addInitScript(() => {
    // minimal invoke mock so the app renders
    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd) => {
        if (cmd === "list_backups") return [];
        if (cmd === "list_code_installs") return [{ id: "default", name: "Default", kind: "default", config_dir: "~/.claude", alias_name: null, managed: false }];
        if (cmd === "list_codex_installs") return [];
        if (cmd === "list_desktop_installs") return [];
        if (cmd && cmd.startsWith("plugin:event|")) return 0;
        if (cmd && cmd.startsWith("list_") && cmd.endsWith("_library")) return [];
        return null;
      },
      transformCallback: () => 1,
      convertFileSrc: (p) => p,
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
    };
    window.__TAURI_METADATA__ = window.__TAURI_INTERNALS__.metadata;
    // mimic exactly what tauri-plugin-decorum injects
    document.addEventListener("DOMContentLoaded", () => {
      const tb = document.createElement("div");
      tb.setAttribute("data-tauri-decorum-tb", "");
      Object.assign(tb.style, { top: "0", left: "0", zIndex: "100", width: "100%", height: "32px", display: "flex", position: "fixed", background: "transparent" });
      const drag = document.createElement("div");
      Object.assign(drag.style, { width: "100%", height: "100%" });
      drag.setAttribute("data-tauri-drag-region", "");
      tb.appendChild(drag);
      document.body.appendChild(tb);
    });
  });

  const page = await ctx.newPage();
  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: "networkidle", timeout: 60000 });
  await page.waitForTimeout(800);

  const overlayExists = await page.locator("[data-tauri-decorum-tb]").count();
  const pe = await page.locator("[data-tauri-decorum-tb]").evaluate((el) => getComputedStyle(el).pointerEvents);

  // What element actually receives a click at the centre of each toolbar button?
  // (elementFromPoint is exactly the hit-test the browser uses to route clicks.)
  const hitTest = async () =>
    page.evaluate(() => {
      const out = {};
      for (const label of ["Backups and restore", "Toggle theme", "Refresh"]) {
        const btn = [...document.querySelectorAll("button")].find(
          (b) => (b.getAttribute("aria-label") || b.textContent || "").includes(label === "Refresh" ? "Refresh" : label),
        );
        if (!btn) { out[label] = "no-button"; continue; }
        const r = btn.getBoundingClientRect();
        const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        // does the hit land on the button or inside it?
        out[label] = btn.contains(hit) || hit === btn ? "button" : (hit?.getAttribute("data-tauri-decorum-tb") != null ? "decorum-overlay" : (hit?.tagName || "other"));
      }
      return out;
    });

  // CONTROL — defeat our fix (override the !important rule) so the overlay eats
  // clicks again, as it did before: hits must land on the decorum overlay.
  await page.locator("[data-tauri-decorum-tb]").evaluate((el) => el.style.setProperty("pointer-events", "auto", "important"));
  const blocked = await hitTest();

  // FIXED — restore our CSS rule (pointer-events:none): hits land on the buttons.
  await page.locator("[data-tauri-decorum-tb]").evaluate((el) => el.style.removeProperty("pointer-events"));
  const fixed = await hitTest();

  // And the click really opens the panel now.
  await page.getByRole("button", { name: "Backups and restore" }).click({ force: false, timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(400);
  const openedWhenFixed = await page.getByText("Backups & Restore").first().isVisible().catch(() => false);

  await browser.close();
  server.close();
  // Before the fix every toolbar button is blocked (the click lands on decorum's
  // overlay/drag div, never the button); after the fix every one resolves to the button.
  const blockedAll = Object.values(blocked).every((v) => v !== "button");
  const fixedAll = Object.values(fixed).every((v) => v === "button");
  const pass = overlayExists === 1 && pe === "none" && blockedAll && fixedAll && openedWhenFixed;
  console.log("VERIFY", JSON.stringify({ overlayExists, cssPointerEvents: pe, blocked, fixed, openedWhenFixed, pass }));
  if (!pass) process.exit(1);
};
start().catch((e) => { console.error(e); process.exit(1); });

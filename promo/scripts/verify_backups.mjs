// Verify the Backups/Restore feature in the REAL built frontend with a mocked
// Tauri invoke: open the Backups panel from the toolbar, screenshot the list,
// then exercise the restore confirm flow.
import { chromium } from "playwright";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.resolve(HERE, "../../frontend/dist");
const SHOTS = path.resolve(HERE, "../shots");
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
    const now = 1750000000000;
    const BACKUPS = [
      { id: "20260622-090000000", createdAtMs: now, reason: "before-apply", label: "Before applying changes · claude_skills", totalFiles: 142, totalBytes: 532000, entries: [] },
      { id: "20260622-085500000", createdAtMs: now - 600000, reason: "startup", label: "Startup safety backup", totalFiles: 138, totalBytes: 511000, entries: [] },
      { id: "20260621-210000000", createdAtMs: now - 86400000, reason: "manual", label: "Manual backup", totalFiles: 130, totalBytes: 498000, entries: [] },
    ];
    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd) => {
        if (cmd === "list_backups") return BACKUPS;
        if (cmd === "restore_backup") return { id: BACKUPS[0].id, createdAtMs: now, restored: 6, relinked: 2 };
        if (cmd === "create_backup") return BACKUPS[0];
        if (cmd === "list_code_installs") return [{ id: "default", name: "Default", kind: "default", config_dir: "~/.claude", alias_name: null, managed: false }];
        if (cmd === "list_codex_installs") return [{ id: "default", name: "Default", kind: "default", data_dir: "~/.codex", app_path: null, launcher_path: null, managed: false, is_running: false }];
        if (cmd === "list_desktop_installs") return [];
        if (cmd && cmd.startsWith("plugin:event|")) return 0; // event listen/unlisten no-op
        if (cmd && cmd.startsWith("list_") && cmd.endsWith("_library")) return [];
        return null;
      },
      transformCallback: (cb) => { window["_cb"] = cb; return 1; },
      convertFileSrc: (p) => p,
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
    };
    window.__TAURI_METADATA__ = window.__TAURI_INTERNALS__.metadata;
  });

  const page = await ctx.newPage();
  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: "networkidle", timeout: 60000 });
  await page.waitForTimeout(900);
  const shot = (n) => page.screenshot({ path: path.join(SHOTS, n + ".png") });

  // Open the Backups panel from the toolbar.
  await page.getByRole("button", { name: "Backups and restore" }).click();
  await page.waitForTimeout(500);
  await shot("verify-backups-list");
  const panelVisible = await page.getByText("Backups & Restore").first().isVisible().catch(() => false);
  const rowCount = await page.getByText(/Before applying|Startup safety|Manual backup/).count();

  // Enter the confirm-restore state on the first snapshot.
  await page.getByRole("button", { name: /^Restore$/ }).first().click().catch(() => {});
  await page.waitForTimeout(300);
  await shot("verify-backups-confirm");
  const confirmVisible = await page.getByRole("button", { name: /Confirm restore/ }).first().isVisible().catch(() => false);

  await browser.close();
  server.close();
  console.log("VERIFY", JSON.stringify({ panelVisible, rowCount, confirmVisible }));
};
start().catch((e) => { console.error(e); process.exit(1); });

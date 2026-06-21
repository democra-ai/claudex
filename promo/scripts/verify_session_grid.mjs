// Verify the per-session share grid: render the real built frontend with a
// mocked Tauri invoke, expand a Codex project row, screenshot the session×account
// grid, then click a session cell to stage a share and screenshot the pending bar.
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
    const codexInstalls = [
      { id: "default", name: "Default", kind: "default", data_dir: "~/.codex", app_path: "/Applications/Codex.app", launcher_path: null, managed: false, is_running: false },
      { id: "profile:judy", name: "judy", kind: "profile", data_dir: "~/.codex-judy", app_path: "/Applications/Codex.app", launcher_path: null, managed: true, is_running: false },
    ];
    const cell = (id, name, state) => ({ install_id: id, install_name: name, data_dir: "x", kind: id === "default" ? "default" : "profile", state, present: state !== "absent", detail: state === "absent" ? null : "2 sessions · 1h ago", digest: null, link_target_digest: null });
    const codexCols = [["default", "Default"], ["profile:judy", "judy"]];
    const rowCells = (states) => codexCols.map((c, i) => cell(c[0], c[1], states[i]));
    const ROWS = {
      list_codex_sessions_library: [
        { id: "__all_sessions__", label: "All Codex sessions", description: "Toggle to symlink the whole ~/.codex/sessions dir.", group: "Sessions", interactive: true, cells: rowCells(["independent", "independent"]) },
        { id: "/Users/tao.shen/democra-ai", label: "democra-ai", description: "~/democra-ai", group: "Projects", interactive: false, cells: rowCells(["independent", "absent"]) },
      ],
    };
    const sCell = (id, name, state, actionable = true) => ({ install_id: id, install_name: name, state, actionable });
    const SHARE_GRID = [
      { session_id: "s1", title: "Redesign the share matrix interaction", cwd: "/Users/tao.shen/democra-ai", model: "gpt-5-codex", last_activity_ms: 1, active: false, cells: [sCell("default", "Default", "independent"), sCell("profile:judy", "judy", "absent")] },
      { session_id: "s2", title: "Fix Codex live-state detection", cwd: "/Users/tao.shen/democra-ai", model: "gpt-5-codex", last_activity_ms: 1, active: false, cells: [sCell("default", "Default", "shared"), sCell("profile:judy", "judy", "shared")] },
      { session_id: "s3", title: "Per-session sharing (in progress now)", cwd: "/Users/tao.shen/democra-ai", model: "gpt-5-codex", last_activity_ms: 9e15, active: true, cells: [sCell("default", "Default", "independent"), sCell("profile:judy", "judy", "absent")] },
    ];
    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd) => {
        if (cmd === "list_codex_installs") return codexInstalls;
        if (cmd === "list_code_installs") return [];
        if (cmd === "list_desktop_installs") return [];
        if (cmd === "list_session_share_grid") return SHARE_GRID;
        if (cmd === "list_codex_sessions_for_project") return [];
        if (cmd in ROWS) return ROWS[cmd];
        if (cmd.startsWith("list_") && cmd.endsWith("_library")) return [];
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
  const click = async (loc) => { try { await loc.first().click({ timeout: 4000 }); await page.waitForTimeout(500); return true; } catch (e) { console.log("miss:", e.message.split("\n")[0]); return false; } };

  // Codex tab → expand the project row via its chevron (all-sessions chevron is
  // index 0, the democra-ai project row is index 1).
  await click(page.getByRole("button", { name: "Codex" }));
  await page.waitForTimeout(400);
  await click(page.getByRole("button", { name: "Toggle sessions" }).nth(1));
  await page.waitForTimeout(800);
  await shot("verify-session-grid");
  const gridVisible = await page.getByText("Share individual sessions").first().isVisible().catch(() => false);
  const activeLock = await page.getByText("active", { exact: true }).first().isVisible().catch(() => false);
  console.log("grid heading visible:", gridVisible, "| active badge visible:", activeLock);

  // Click an absent cell on session s1 (judy column) to stage a share.
  const cells = page.locator('button[title*="Click to share with judy"]');
  const n = await cells.count();
  if (n > 0) { await cells.first().click(); await page.waitForTimeout(500); }
  await shot("verify-session-grid-pending");
  const applyVisible = await page.getByRole("button", { name: /Apply/i }).first().isVisible().catch(() => false);
  console.log("after session-cell click → Apply visible:", applyVisible, "| share cells found:", n);

  await browser.close();
  server.close();
  console.log("VERIFY", JSON.stringify({ gridVisible, activeLock, applyVisible, shareCells: n }));
};
start().catch((e) => { console.error(e); process.exit(1); });

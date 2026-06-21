// Verify the synthetic "All" row: render the real frontend with mocked rows for
// a per-item kind (Codex MCPs), confirm an "All" row appears above the items,
// then click its cell to stage a share-all and confirm the pending bar.
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
    const cell = (id, name, state) => ({ install_id: id, install_name: name, data_dir: "x", kind: id === "default" ? "default" : "profile", state, present: state !== "absent", detail: null, digest: null, link_target_digest: null });
    const cols = [["default", "Default"], ["profile:judy", "judy"]];
    const row = (id, label, states) => ({ id, label, description: id, group: "MCP", interactive: true, cells: cols.map((c, i) => cell(c[0], c[1], states[i])) });
    const ROWS = {
      list_codex_mcp_library: [
        row("github", "github", ["copied", "copied"]),
        row("playwright", "playwright", ["independent", "absent"]),
        row("filesystem", "filesystem", ["independent", "absent"]),
      ],
    };
    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd) => {
        if (cmd === "list_codex_installs") return codexInstalls;
        if (cmd === "list_code_installs") return [];
        if (cmd === "list_desktop_installs") return [];
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
  const click = async (loc) => { try { await loc.first().click({ timeout: 4000 }); await page.waitForTimeout(450); return true; } catch (e) { console.log("miss:", e.message.split("\n")[0]); return false; } };

  await click(page.getByRole("button", { name: "Codex" }));
  await page.waitForTimeout(300);
  // Go to the MCPs kind.
  await click(page.getByText("MCPs", { exact: true }));
  await page.waitForTimeout(500);
  await shot("verify-all-row");
  const allVisible = await page.getByText("All", { exact: true }).first().isVisible().catch(() => false);

  // Click the All row's judy cell (its aria-label/tooltip mentions judy + state).
  // The All row is the first interactive row; click its 2nd account cell.
  const cells = page.getByRole("button", { name: /Independent|Absent|Shared|Copied|Diverged/ });
  const n = await cells.count();
  // The All row cells render first; click one that previews a share.
  if (n > 1) { await cells.nth(1).click().catch(() => {}); await page.waitForTimeout(400); }
  await shot("verify-all-row-pending");
  const applyVisible = await page.getByRole("button", { name: /Apply/i }).first().isVisible().catch(() => false);
  console.log("All row visible:", allVisible, "| cells:", n, "| Apply after click:", applyVisible);

  await browser.close();
  server.close();
  console.log("VERIFY", JSON.stringify({ allVisible, applyVisible }));
};
start().catch((e) => { console.error(e); process.exit(1); });

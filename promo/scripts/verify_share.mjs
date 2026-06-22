// Verify the share-matrix interaction redesign by rendering the REAL built
// frontend with a mocked Tauri invoke, then exercising a cell click:
//   verify-01-matrix       → column headers (no ■ square, green dot) + legend
//   verify-02-pending      → click an INDEPENDENT cell → previews shared
//                            (green dot) + amber pending ring + pending bar
//   verify-03-toggleback   → click the SAME cell again → pending cleared
import { chromium } from "playwright";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.resolve(HERE, "../../frontend/dist");
const SHOTS = path.resolve(HERE, "../shots");
fs.mkdirSync(SHOTS, { recursive: true });

const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".json": "application/json" };
function serve() {
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      let p = decodeURIComponent(req.url.split("?")[0]);
      if (p === "/") p = "/index.html";
      let file = path.join(DIST, p);
      if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) file = path.join(DIST, "index.html");
      const ext = path.extname(file);
      res.writeHead(200, { "Content-Type": MIME[ext] || "application/octet-stream" });
      fs.createReadStream(file).pipe(res);
    });
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

const DATA = {
  code_installs: [
    { id: "default", name: "Default", kind: "default", config_dir: "~/.claude", alias_name: null, managed: false },
    { id: "work", name: "Work", kind: "profile", config_dir: "~/.claude-work", alias_name: "work", managed: true },
    { id: "personal", name: "Personal", kind: "profile", config_dir: "~/.claude-personal", alias_name: "personal", managed: true },
  ],
  codex_installs: [{ id: "default", name: "Default", kind: "default", data_dir: "~/.codex", app_path: "/Applications/Codex.app", launcher_path: null, managed: false, is_running: false }],
  desktop_installs: [{ id: "default", name: "Default", kind: "default", data_dir: "~/Library/Application Support/Claude", app_path: "/Applications/Claude.app", launcher_path: null, managed: false, is_running: true }],
};

const start = async () => {
  const server = await serve();
  const { port } = server.address();
  const base = `http://127.0.0.1:${port}/`;
  const browser = await chromium.launch({ channel: "chrome" });
  const ctx = await browser.newContext({ viewport: { width: 1480, height: 940 }, deviceScaleFactor: 2 });

  await ctx.addInitScript((data) => {
    const cell = (id, name, dir, state) => ({ install_id: id, install_name: name, data_dir: dir, kind: id === "default" ? "default" : "profile", state, present: state !== "absent", detail: null, digest: state === "shared" ? "a1b2c3d4" : null, link_target_digest: state === "shared" ? "a1b2c3d4" : null });
    const cols = [["default", "Default ~/.claude", "~/.claude"], ["work", "Work", "~/.claude-work"], ["personal", "Personal", "~/.claude-personal"]];
    const rowCells = (states) => cols.map((c, i) => cell(c[0], c[1], c[2], states[i]));
    const ROWS = {
      list_claude_sessions_library: [
        { id: "p1", label: "acme-storefront", description: "42 sessions · active 2h ago", group: "PROJECTS", interactive: true, cells: rowCells(["independent", "shared", "shared"]) },
        { id: "p2", label: "billing-service", description: "8 sessions · yesterday", group: "PROJECTS", interactive: true, cells: rowCells(["shared", "shared", "shared"]) },
        { id: "p3", label: "mobile-app", description: "15 sessions · 3d ago", group: "PROJECTS", interactive: true, cells: rowCells(["independent", "independent", "absent"]) },
      ],
    };
    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd, args) => {
        if (cmd === "list_code_installs") return data.code_installs;
        if (cmd === "list_codex_installs") return data.codex_installs;
        if (cmd === "list_desktop_installs") return data.desktop_installs;
        if (cmd in ROWS) return ROWS[cmd];
        if (cmd.startsWith("list_") && cmd.endsWith("_library")) return [];
        return null;
      },
      transformCallback: (cb) => { const id = 12345; window["_cb" + id] = cb; return id; },
      convertFileSrc: (p) => p,
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
    };
    window.__TAURI_METADATA__ = window.__TAURI_INTERNALS__.metadata;
  }, DATA);

  const page = await ctx.newPage();
  await page.goto(base, { waitUntil: "networkidle", timeout: 60000 });
  await page.waitForTimeout(1200);
  const shot = (n) => page.screenshot({ path: path.join(SHOTS, n + ".png") });

  // 1) matrix header + legend
  await shot("verify-01-matrix");

  // 2) click an INDEPENDENT cell → should preview shared + amber ring + pending bar
  const indep = page.getByRole("button", { name: /Independent/ }).first();
  const box = await indep.boundingBox();
  if (!box) { console.error("no Independent cell found"); process.exit(1); }
  const cx = box.x + box.width / 2, cy = box.y + box.height / 2;
  await page.mouse.click(cx, cy);
  await page.waitForTimeout(600);
  await shot("verify-02-pending");
  // assert a pending bar / Apply control appeared
  const applyVisible = await page.getByRole("button", { name: /Apply/i }).first().isVisible().catch(() => false);
  console.log("after click → Apply visible:", applyVisible);

  // 3) click SAME location again → toggle-back, pending should clear
  await page.mouse.click(cx, cy);
  await page.waitForTimeout(600);
  await shot("verify-03-toggleback");
  const applyGone = !(await page.getByRole("button", { name: /Apply/i }).first().isVisible().catch(() => false));
  console.log("after 2nd click → Apply gone (toggle-back):", applyGone);

  await browser.close();
  server.close();
  console.log("VERIFY_RESULT", JSON.stringify({ pendingAppeared: applyVisible, toggleBackCleared: applyGone }));
};
start().catch((e) => { console.error(e); process.exit(1); });

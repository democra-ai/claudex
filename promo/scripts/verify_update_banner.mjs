// Verify the auto-update banner in the REAL built frontend with a mocked Tauri
// updater: mock plugin:updater|check to report an available update and confirm
// the banner renders with the version + "Install & restart".
import { chromium } from "playwright";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.resolve(HERE, "../../frontend/dist");
const SHOTS = path.resolve(HERE, "../shots");
const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml", ".woff2": "font/woff2" };
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
    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd) => {
        // updater: report an available newer version
        if (cmd === "plugin:updater|check") return { rid: 1, currentVersion: "0.7.0", version: "0.8.0", date: "2026-06-23", body: "Bug fixes and improvements" };
        if (cmd === "list_code_installs") return [{ id: "default", name: "Default", kind: "default", config_dir: "~/.claude", alias_name: null, managed: false }];
        if (cmd === "list_codex_installs") return [];
        if (cmd === "list_desktop_installs") return [];
        if (cmd === "list_backups") return [];
        if (cmd && cmd.startsWith("plugin:event|")) return 0;
        if (cmd && cmd.startsWith("list_") && cmd.endsWith("_library")) return [];
        return null;
      },
      transformCallback: () => 1,
      convertFileSrc: (p) => p,
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
    };
    window.__TAURI_METADATA__ = window.__TAURI_INTERNALS__.metadata;
  });

  const page = await ctx.newPage();
  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: "networkidle", timeout: 60000 });
  await page.waitForTimeout(1200); // let the launch check() resolve
  await page.screenshot({ path: path.join(SHOTS, "verify-update-banner.png") });

  const bannerVisible = await page.getByText(/is available/).first().isVisible().catch(() => false);
  const showsVersion = await page.getByText(/Claudex 0\.8\.0/).first().isVisible().catch(() => false);
  const hasInstall = await page.getByRole("button", { name: /Install & restart/i }).first().isVisible().catch(() => false);

  await browser.close();
  server.close();
  const pass = bannerVisible && showsVersion && hasInstall;
  console.log("VERIFY", JSON.stringify({ bannerVisible, showsVersion, hasInstall, pass }));
  if (!pass) process.exit(1);
};
start().catch((e) => { console.error(e); process.exit(1); });

// Capture REAL Claudex UI states by serving the built frontend/dist and
// injecting a mock window.__TAURI_INTERNALS__.invoke. The actual React
// components / CSS / theming render — only the backend data is stubbed.
import { chromium } from "playwright";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.resolve(HERE, "../../frontend/dist");
const SHOTS = path.resolve(HERE, "../shots");
fs.mkdirSync(SHOTS, { recursive: true });

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".json": "application/json",
};

function serve() {
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      let p = decodeURIComponent(req.url.split("?")[0]);
      if (p === "/") p = "/index.html";
      let file = path.join(DIST, p);
      if (!fs.existsSync(file) || fs.statSync(file).isDirectory())
        file = path.join(DIST, "index.html"); // SPA fallback
      const ext = path.extname(file);
      res.writeHead(200, { "Content-Type": MIME[ext] || "application/octet-stream" });
      fs.createReadStream(file).pipe(res);
    });
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

// ---- mock backend data (realistic) ----
const DATA = {
  code_installs: [
    { id: "default", name: "Default", kind: "default", config_dir: "~/.claude", alias_name: null, managed: false },
    { id: "work", name: "Work", kind: "profile", config_dir: "~/.claude-work", alias_name: "work", managed: true },
    { id: "personal", name: "Personal", kind: "profile", config_dir: "~/.claude-personal", alias_name: "personal", managed: true },
  ],
  codex_installs: [
    { id: "default", name: "Default", kind: "default", data_dir: "~/.codex", app_path: "/Applications/Codex.app", launcher_path: null, managed: false, is_running: false },
    { id: "research", name: "Research", kind: "profile", data_dir: "~/.codex-research", app_path: "/Applications/Codex.app", launcher_path: null, managed: true, is_running: true },
  ],
  desktop_installs: [
    { id: "default", name: "Default", kind: "default", data_dir: "~/Library/Application Support/Claude", app_path: "/Applications/Claude.app", launcher_path: null, managed: false, is_running: true },
    { id: "work", name: "Work", kind: "profile", data_dir: "~/Library/Application Support/Claude-work", app_path: "/Applications/Claude.app", launcher_path: null, managed: true, is_running: false },
    { id: "personal", name: "Personal", kind: "profile", data_dir: "~/Library/Application Support/Claude-personal", app_path: "/Applications/Claude.app", launcher_path: null, managed: true, is_running: false },
  ],
};

const start = async () => {
  const server = await serve();
  const { port } = server.address();
  const base = `http://127.0.0.1:${port}/`;
  console.log("serving", DIST, "→", base);

  const browser = await chromium.launch({ channel: "chrome" });
  const ctx = await browser.newContext({ viewport: { width: 1480, height: 940 }, deviceScaleFactor: 2 });

  await ctx.addInitScript((data) => {
    const cell = (id, name, dir, state) => ({
      install_id: id, install_name: name, data_dir: dir, kind: id === "default" ? "default" : "profile",
      state, present: state !== "absent", detail: null, digest: state === "shared" ? "a1b2c3d4" : null,
      link_target_digest: state === "shared" ? "a1b2c3d4" : null,
    });
    const claudeCols = [["default", "Default ~/.claude", "~/.claude"], ["work", "Work", "~/.claude-work"], ["personal", "Personal", "~/.claude-personal"]];
    const codexCols = [["default", "Default", "~/.codex"], ["research", "Research", "~/.codex-research"]];
    const rowCells = (cols, states) => cols.map((c, i) => cell(c[0], c[1], c[2], states[i]));

    const ROWS = {
      // Claude tab — Code Sessions (orange)
      list_claude_sessions_library: [
        { id: "p1", label: "acme-storefront", description: "42 sessions · active 2h ago", group: "PROJECTS", interactive: true, cells: rowCells(claudeCols, ["independent", "shared", "shared"]) },
        { id: "p2", label: "billing-service", description: "8 sessions · yesterday", group: "PROJECTS", interactive: true, cells: rowCells(claudeCols, ["independent", "independent", "absent"]) },
        { id: "p3", label: "mobile-app", description: "15 sessions · 3d ago", group: "PROJECTS", interactive: true, cells: rowCells(claudeCols, ["shared", "shared", "absent"]) },
        { id: "p4", label: "blog-cms", description: "6 sessions · last week", group: "PROJECTS", interactive: true, cells: rowCells(claudeCols, ["absent", "independent", "independent"]) },
      ],
      // Claude tab — Memory (CLAUDE.md)
      list_claude_memory_library: [
        { id: "m1", label: "Global · CLAUDE.md", description: "~/.claude/CLAUDE.md · 1.2 KB", group: "MEMORY", interactive: true, cells: [cell("default", "Default", "~/.claude/CLAUDE.md", "independent"), cell("work", "Work", "~/.claude-work/CLAUDE.md", "shared"), cell("personal", "Personal", "~/.claude-personal/CLAUDE.md", "shared")] },
        { id: "m2", label: "acme-storefront/CLAUDE.md", description: "project memory · 3.4 KB", group: "MEMORY", interactive: true, cells: [cell("default", "Default", "~/.claude/CLAUDE.md", "independent"), cell("work", "Work", "~/.claude-work/CLAUDE.md", "absent"), cell("personal", "Personal", "~/.claude-personal/CLAUDE.md", "absent")] },
      ],
      // Claude tab — Skills (within Claude)
      list_claude_skills_library: [
        { id: "pdf", label: "pdf", description: "Fill & extract PDF forms", group: "SKILLS", interactive: true, cells: rowCells(claudeCols, ["shared", "shared", "absent"]) },
        { id: "dataviz", label: "data-visualization", description: "Charts via visx / d3-shape", group: "SKILLS", interactive: true, cells: rowCells(claudeCols, ["independent", "absent", "absent"]) },
      ],
      // Codex tab — sessions (indigo)
      list_codex_sessions_library: [
        { id: "c1", label: "weather-api", description: "12 sessions · active now", group: "PROJECTS", interactive: true, cells: rowCells(codexCols, ["independent", "shared"]) },
        { id: "c2", label: "todo-app", description: "5 sessions · 2d ago", group: "PROJECTS", interactive: true, cells: rowCells(codexCols, ["shared", "shared"]) },
        { id: "c3", label: "image-resizer", description: "3 sessions · last week", group: "PROJECTS", interactive: true, cells: rowCells(codexCols, ["independent", "absent"]) },
      ],
      list_codex_skills_library: [
        { id: "pdf", label: "pdf", description: "shared from Claude", group: "SKILLS", interactive: true, cells: rowCells(codexCols, ["shared", "shared"]) },
      ],
      list_codex_mcp_library: [
        { id: "github", label: "github", description: "stdio · @modelcontextprotocol/server-github", group: "MCP", interactive: true, cells: rowCells(codexCols, ["copied", "copied"]) },
      ],
      list_codex_memory_library: [
        { id: "agents", label: "AGENTS.md", description: "~/.codex/AGENTS.md", group: "MEMORY", interactive: true, cells: rowCells(codexCols, ["independent", "shared"]) },
      ],
      list_codex_preferences_library: [
        { id: "model", label: "model", description: "gpt-5-codex", group: "MODEL", interactive: true, cells: rowCells(codexCols, ["diverged", "independent"]) },
        { id: "approval_policy", label: "approval_policy", description: "on-request", group: "SANDBOX", interactive: true, cells: rowCells(codexCols, ["copied", "copied"]) },
      ],
      // Claude tab — MCP / Preferences (desktop columns)
      list_library_mcp: [
        { id: "github", label: "github", description: "npx @modelcontextprotocol/server-github", group: "MCP", interactive: true, cells: [cell("default", "Default", "x", "copied"), cell("work", "Work", "x", "copied")] },
      ],
      list_library_preferences: [
        { id: "theme", label: "theme", description: "dark", group: "UI", interactive: true, cells: [cell("default", "Default", "x", "copied"), cell("work", "Work", "x", "copied")] },
      ],
      // Share tab — Skills cross-tool (green glow across Claude + Codex)
      list_skills_library: [
        { id: "pdf", label: "pdf", description: "Fill & extract PDF forms", group: "SHARED SKILLS", interactive: true, cells: [cell("default", "Default ~/.claude", "~/.claude/skills", "shared"), cell("work", "Work", "~/.claude-work/skills", "shared"), cell("personal", "Personal", "~/.claude-personal/skills", "absent"), cell("codex:global", "Codex", "~/.codex/skills", "shared")] },
        { id: "dataviz", label: "data-visualization", description: "Charts via visx / d3-shape", group: "SHARED SKILLS", interactive: true, cells: [cell("default", "Default ~/.claude", "~/.claude/skills", "shared"), cell("work", "Work", "~/.claude-work/skills", "absent"), cell("personal", "Personal", "~/.claude-personal/skills", "absent"), cell("codex:global", "Codex", "~/.codex/skills", "shared")] },
        { id: "webresearch", label: "web-research", description: "Multi-source research agent", group: "SHARED SKILLS", interactive: true, cells: [cell("default", "Default ~/.claude", "~/.claude/skills", "independent"), cell("work", "Work", "~/.claude-work/skills", "independent"), cell("personal", "Personal", "~/.claude-personal/skills", "absent"), cell("codex:global", "Codex", "~/.codex/skills", "absent")] },
        { id: "slackgif", label: "slack-gif-creator", description: "Make Slack reaction GIFs", group: "SHARED SKILLS", interactive: true, cells: [cell("default", "Default ~/.claude", "~/.claude/skills", "absent"), cell("work", "Work", "~/.claude-work/skills", "absent"), cell("personal", "Personal", "~/.claude-personal/skills", "absent"), cell("codex:global", "Codex", "~/.codex/skills", "independent")] },
      ],
      // Share tab — MCP cross-tool (Claude Code ↔ Codex, copy mode)
      list_mcp_cross_library: [
        { id: "github", label: "github", description: "@modelcontextprotocol/server-github", group: "MCP SERVERS", interactive: true, cells: [cell("claude:code", "Claude Code", "~/.claude.json", "copied"), cell("codex:global", "Codex", "~/.codex/config.toml", "copied")] },
        { id: "playwright", label: "playwright", description: "@playwright/mcp", group: "MCP SERVERS", interactive: true, cells: [cell("claude:code", "Claude Code", "~/.claude.json", "independent"), cell("codex:global", "Codex", "~/.codex/config.toml", "absent")] },
        { id: "filesystem", label: "filesystem", description: "server-filesystem ~/projects", group: "MCP SERVERS", interactive: true, cells: [cell("claude:code", "Claude Code", "~/.claude.json", "copied"), cell("codex:global", "Codex", "~/.codex/config.toml", "copied")] },
      ],
      // Share tab — Memory cross-tool (CLAUDE.md ↔ AGENTS.md)
      list_memory_library: [
        { id: "global", label: "Global memory", description: "CLAUDE.md ↔ AGENTS.md", group: "MEMORY", interactive: true, cells: [cell("claude-code:default", "Default ~/.claude", "~/.claude/CLAUDE.md", "shared"), cell("claude-code:work", "Work", "~/.claude-work/CLAUDE.md", "shared"), cell("claude-code:personal", "Personal", "~/.claude-personal/CLAUDE.md", "absent"), cell("codex:default", "Codex Default", "~/.codex/AGENTS.md", "shared"), cell("codex:research", "Codex Research", "~/.codex-research/AGENTS.md", "absent")] },
      ],
    };

    const CLAUDE_MD = `# Working agreements

- Prefer TypeScript; keep functions small and pure.
- Write a test for every new endpoint before shipping.
- Run lint + typecheck before committing.
- Conventional commits; one logical change per PR.
- Never log secrets; read them from the environment.
- Keep PRs under ~400 lines; split larger work.

## Stack
- Web: Next.js + TypeScript + Tailwind.
- Data: Postgres via Prisma; migrations in /db.
- Auth: session cookies, httpOnly + SameSite=Lax.
- Deploy: Vercel preview per PR, prod on main.

## Conventions
- Components in PascalCase; hooks prefixed with use.
- Colocate tests as *.test.ts next to the source.
- API routes return typed JSON; no any at boundaries.
- Format with Prettier on save; no manual alignment.
`;
    const MCP_JSON = `{
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-github"],
  "env": {
    "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_••••••••••••••••"
  }
}`;
    const STATS = (id) => ({
      install_id: id, install_name: id, kind: id === "default" ? "default" : "profile", data_dir: "~/.claude-" + id,
      account_id: "acct_" + id, org_id: "org_x", identities: [{ account_id: "acct_" + id, is_owner: true, account_name: id, email_address: id + "@example.com", agent_session_count: 42, last_activity_ms: 0 }],
      tokens_today: 1840000, tokens_today_date: "2026-06-17", code_sessions_last_5h: 4, code_sessions_last_24h: 11, code_sessions_last_7d: 63, code_sessions_last_30d: 240,
      code_sessions_per_day_baseline: 9, code_sessions_today: 7, top_model_last_7d: "opus-4-8", device_id: "MacBook", ssh_remote_count: 0, disk_bytes: 1280000000,
      code_panel_bytes: null, cowork_agent_bytes: null, created_at_ms: 0, last_activity_ms: 0, code_session_count: 240, code_total_bytes: 1280000000,
      code_recent_cwds: ["~/code/acme-storefront"], cowork_session_count: 0, extension_count: 3, mcp_server_count: 5, cowork_skill_count: 4, link_group: "a1b2c3d4", shared_with: ["work", "personal"],
    });

    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd, args) => {
        if (cmd === "list_code_installs") return data.code_installs;
        if (cmd === "list_codex_installs") return data.codex_installs;
        if (cmd === "list_desktop_installs") return data.desktop_installs;
        if (cmd === "get_profile_stats") return STATS((args && args.installId) || "default");
        if (cmd in ROWS) return ROWS[cmd];
        if (cmd.startsWith("list_") && cmd.endsWith("_library")) return [];
        if (cmd === "read_text_file") return CLAUDE_MD;
        if (cmd === "read_mcp_server") return MCP_JSON;
        if (cmd === "list_claude_sessions_for_project" || cmd === "list_codex_sessions_for_project" || cmd === "list_sessions_for_project")
          return [
            { session_id: "s1", title: "Add user login flow", cwd: "~/code/acme-storefront", process_name: "claude", model: "opus-4-8", created_at_ms: 0, last_activity_ms: 0, account_name: "work", email_address: null },
            { session_id: "s2", title: "Fix checkout total rounding", cwd: "~/code/acme-storefront", process_name: "claude", model: "opus-4-8", created_at_ms: 0, last_activity_ms: 0, account_name: "work", email_address: null },
            { session_id: "s3", title: "Refactor the API client", cwd: "~/code/acme-storefront", process_name: "claude", model: "sonnet-4-6", created_at_ms: 0, last_activity_ms: 0, account_name: "personal", email_address: null },
          ];
        if (cmd === "get_session_transcript") return "user: add a login form with email + password\nassistant: done — added the form, validation, and a session cookie...";
        return null;
      },
      transformCallback: (cb) => { const id = Math.floor(Math.random() * 1e9); window["_cb" + id] = cb; return id; },
      convertFileSrc: (p) => p,
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
    };
    window.__TAURI_METADATA__ = window.__TAURI_INTERNALS__.metadata;
  }, DATA);

  const page = await ctx.newPage();
  await page.goto(base, { waitUntil: "networkidle", timeout: 60000 });
  await page.waitForTimeout(1200);
  const shot = (n) => page.screenshot({ path: path.join(SHOTS, n + ".png") });
  const sheet = () => page.locator("aside.border-l");
  const shotSheet = (n) => sheet().screenshot({ path: path.join(SHOTS, n + ".png") });
  const click = async (loc, label) => { try { await loc.first().click({ timeout: 5000 }); await page.waitForTimeout(650); return true; } catch (e) { console.log("click miss:", label, e.message.split("\n")[0]); return false; } };
  const tab = (name) => page.getByRole("button", { name }).first();
  const nav = (name) => page.getByText(name, { exact: true });

  // ---- Claude (orange) ----
  await shot("01-claude-sessions");

  // Claude → Sessions row detail (manage: view / delete / import sessions)
  await click(page.getByText("acme-storefront"), "sessions row");
  await page.waitForTimeout(500);
  await shot("06-claude-sessions-detail");
  await shotSheet("06b-sessions-sheet");
  await click(page.locator('aside.border-l').getByText("× close"), "close"); // best-effort
  await page.keyboard.press("Escape").catch(() => {});

  // Claude → Memory → open editor
  await click(nav("Memory"), "memory nav");
  await shot("02-claude-memory");
  await click(page.getByText("Global · CLAUDE.md"), "memory row");
  await page.waitForTimeout(400);
  await click(page.locator("aside.border-l").getByText("view · edit").first(), "memory cell");
  await page.waitForTimeout(500);
  await shotSheet("07-memory-editor");
  await shot("07b-memory-editor-full");
  await page.keyboard.press("Escape").catch(() => {});

  // ---- Codex (indigo) ----
  await click(tab("Codex"), "codex tab");
  await shot("03-codex-sessions");

  // ---- Share (green) ----
  await click(tab("Share"), "share tab");
  await shot("04-share-skills");

  // Share → MCP servers → open JSON editor
  await click(nav("MCP servers"), "mcp nav");
  await shot("05-share-mcp");
  await click(page.getByText("github").first(), "mcp row");
  await page.waitForTimeout(400);
  await click(page.locator("aside.border-l").getByText("view · edit").first(), "mcp cell");
  await page.waitForTimeout(500);
  await shotSheet("08-mcp-editor");
  await shot("08b-mcp-editor-full");

  await browser.close();
  server.close();
  console.log("capture done →", SHOTS);
};
start().catch((e) => { console.error(e); process.exit(1); });

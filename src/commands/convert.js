// `claude-multiprofile convert` — import a session from one tool into the
// other as a brand-new, resumable session.
//
//   convert codex claude [session]   Codex rollout  -> new Claude Code session
//   convert claude codex [session]   Claude session -> new Codex rollout
//
// `session` may be a file path, a session-id (UUID, substring ok), or a
// thread-name substring. Omit it to use the most recent session of the
// source tool. With no args at all, an interactive picker runs.
//
// See src/convert.js for why this is "import as a new session" rather than
// a lossless sync (DAG vs linear threading, provider-private reasoning).

import fs from "node:fs";
import path from "node:path";
import { select } from "@inquirer/prompts";
import { ok, err, info } from "../util.js";
import {
  codexToClaude,
  claudeToCodex,
  resolveCodexSession,
  resolveClaudeSession,
  parseCodexRollout,
  parseClaudeSession,
  DEFAULT_CODEX_HOME,
  DEFAULT_CLAUDE_HOME,
} from "../convert.js";

const TOOLS = ["claude", "codex"];

function recentClaudeSessions(limit = 12) {
  const projects = path.join(DEFAULT_CLAUDE_HOME, "projects");
  if (!fs.existsSync(projects)) return [];
  const rows = [];
  for (const slug of fs.readdirSync(projects)) {
    const dir = path.join(projects, slug);
    let st;
    try {
      st = fs.statSync(dir);
    } catch {
      continue;
    }
    if (!st.isDirectory()) continue;
    for (const f of fs.readdirSync(dir)) {
      if (!f.endsWith(".jsonl")) continue;
      const full = path.join(dir, f);
      rows.push({ id: f.replace(/\.jsonl$/, ""), file: full, mtime: fs.statSync(full).mtimeMs });
    }
  }
  return rows.sort((a, b) => b.mtime - a.mtime).slice(0, limit);
}

function recentCodexSessions(limit = 12) {
  const idxPath = path.join(DEFAULT_CODEX_HOME, "session_index.jsonl");
  if (!fs.existsSync(idxPath)) return [];
  const rows = fs
    .readFileSync(idxPath, "utf8")
    .split("\n")
    .filter(Boolean)
    .map((l) => {
      try {
        return JSON.parse(l);
      } catch {
        return null;
      }
    })
    .filter(Boolean);
  return rows
    .sort((a, b) => String(b.updated_at).localeCompare(String(a.updated_at)))
    .slice(0, limit);
}

async function pickDirection() {
  const from = await select({
    message: "Import a session FROM which tool?",
    choices: [
      { name: "Codex   → into Claude Code", value: "codex" },
      { name: "Claude Code → into Codex", value: "claude" },
    ],
  });
  return { from, to: from === "codex" ? "claude" : "codex" };
}

async function pickSession(from) {
  if (from === "codex") {
    const rows = recentCodexSessions();
    if (!rows.length) return undefined; // fall back to latest-on-disk
    return select({
      message: "Which Codex session?",
      choices: rows.map((r) => ({
        name: `${r.thread_name || "(untitled)"}  ·  ${String(r.updated_at).slice(0, 10)}`,
        value: r.id,
      })),
    });
  }
  const rows = recentClaudeSessions();
  if (!rows.length) return undefined;
  return select({
    message: "Which Claude Code session?",
    choices: rows.map((r) => {
      let title = r.id.slice(0, 8);
      try {
        const ir = parseClaudeSession(r.file);
        const t = ir.turns.find((x) => x.role === "user");
        const tx = t && t.blocks.find((b) => b.kind === "text");
        if (tx) title = tx.text.slice(0, 50).replace(/\s+/g, " ").trim();
      } catch {
        /* ignore */
      }
      return { name: `${title}  ·  ${r.id.slice(0, 8)}`, value: r.file };
    }),
  });
}

export async function convert(args = []) {
  let [from, to, session] = args;

  // Interactive when direction is absent or invalid.
  if (!from || !TOOLS.includes(from) || !to || !TOOLS.includes(to)) {
    ({ from, to } = await pickDirection());
    session = await pickSession(from);
  }

  if (from === to) return err("Source and target tools must differ (claude vs codex).");

  if (from === "codex" && to === "claude") {
    const preview = resolveCodexSession(session, DEFAULT_CODEX_HOME);
    if (!preview) return err(`No Codex session found for: ${session ?? "(latest)"}`);
    const ir = parseCodexRollout(preview);
    info(`Source: ${path.basename(preview)}  (${ir.turns.length} turns, cwd ${ir.cwd || "?"})`);
    const r = codexToClaude(session, {});
    ok(`Imported into a new Claude Code session.`);
    console.log("");
    console.log(`  session id : ${r.sessionId}`);
    console.log(`  written    : ${r.path.replace(process.env.HOME, "~")}`);
    console.log(`  turns      : ${r.turns}`);
    console.log("");
    console.log(`  Resume it with:`);
    console.log(`    cd ${r.cwd}`);
    console.log(`    claude --resume ${r.sessionId}`);
    console.log("");
    info("Lossy by design: reasoning shown as text (crypto payload dropped); alternate branches flattened.");
    return;
  }

  if (from === "claude" && to === "codex") {
    const preview = resolveClaudeSession(session, DEFAULT_CLAUDE_HOME);
    if (!preview) return err(`No Claude session found for: ${session ?? "(latest)"}`);
    const ir = parseClaudeSession(preview);
    info(`Source: ${path.basename(preview)}  (${ir.turns.length} turns, cwd ${ir.cwd || "?"})`);
    const r = claudeToCodex(session, {});
    ok(`Imported into a new Codex session.`);
    console.log("");
    console.log(`  session id : ${r.sessionId}`);
    console.log(`  written    : ${r.path.replace(process.env.HOME, "~")}`);
    console.log(`  turns      : ${r.turns}`);
    console.log("");
    console.log(`  Resume it with:`);
    console.log(`    codex resume ${r.sessionId}`);
    console.log("");
    info(
      "Best-effort: written to rollout + session_index. Codex's TUI picker also reads SQLite, " +
        "so it may not appear in the list until Codex re-indexes. Codex 0.139.0+ has an official " +
        "`/import` for Claude→Codex — prefer it when available."
    );
    return;
  }

  return err(`Unsupported direction: ${from} -> ${to}`);
}

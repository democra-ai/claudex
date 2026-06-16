// Cross-tool session conversion.
//
// Goal (per the design we settled on): NOT a perfect bidirectional sync —
// just "import the other tool's conversation as a brand-new session on my
// side." A Codex rollout becomes a fresh Claude Code session you can
// `claude --resume`; a Claude Code session becomes a fresh Codex rollout.
//
// Why a new session instead of sync: the two on-disk formats agree on the
// easy middle (text turns, tool calls) but diverge at the hard edges —
//   * threading: Claude is a parentUuid DAG, Codex is a linear log;
//   * tool-call ids: toolu_* vs call_*;
//   * reasoning: Codex's encrypted_content / Claude's thinking.signature
//     are provider-private and non-portable.
// So we convert through a lossy intermediate representation (IR) that keeps
// what transfers cleanly (who said what, which tool ran, its output) and
// drops what can't (crypto reasoning payloads, alternate DAG branches).
//
// Directions:
//   codex  -> claude   CLEAN. We only ever WRITE a new *.jsonl under
//                      ~/.claude/projects/<slug>/. Claude indexes sessions
//                      straight from JSONL, so the result is resumable with
//                      no database surgery.
//   claude -> codex    BEST-EFFORT. We write a rollout-*.jsonl + a
//                      session_index.jsonl entry. Codex ALSO keeps a live
//                      SQLite index (state_5.sqlite "threads" table) that
//                      its TUI picker reads, which we deliberately do not
//                      touch (undocumented, version-volatile, lock-prone).
//                      So the converted session is resumable by id but may
//                      not appear in the picker until Codex re-indexes.
//                      Codex 0.139.0+ has an official `/import` for this
//                      direction — prefer it when available.

import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { randomUUID, randomBytes } from "node:crypto";

const HOME = os.homedir();

export const DEFAULT_CLAUDE_HOME = path.join(HOME, ".claude");
export const DEFAULT_CODEX_HOME = process.env.CODEX_HOME || path.join(HOME, ".codex");

// ---------------------------------------------------------------------------
// Intermediate representation
// ---------------------------------------------------------------------------
//
// ir = {
//   source: "claude" | "codex",
//   cwd: string,
//   model: string | null,
//   title: string | null,
//   turns: Turn[]
// }
// Turn  = { role: "user" | "assistant", blocks: Block[] }
// Block =
//   { kind: "text",        text: string }
//   { kind: "reasoning",   text: string }                       // assistant only
//   { kind: "tool_use",    id: string, name: string, input: any } // assistant only
//   { kind: "tool_result", forId: string, output: string, isError: boolean } // user only
//
// `id`/`forId` carry the SOURCE tool-call id (call_* or toolu_*). Emitters
// re-mint ids in the target namespace and keep a bijection so a result still
// points at its call.

/** macOS/Claude project-dir slug: every "/" and "." in the cwd becomes "-". */
export function claudeSlug(cwd) {
  return cwd.replace(/[/.]/g, "-");
}

function newTooluId() {
  return "toolu_" + randomBytes(12).toString("hex");
}

function tryParseJSON(s) {
  if (typeof s !== "string") return s;
  try {
    return JSON.parse(s);
  } catch {
    return { raw: s };
  }
}

function asText(content) {
  // Codex message content is an array of {type, text}; Claude blocks too.
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((p) => (typeof p === "string" ? p : p && typeof p.text === "string" ? p.text : ""))
    .filter(Boolean)
    .join("");
}

// ---------------------------------------------------------------------------
// Codex rollout  ->  IR
// ---------------------------------------------------------------------------

const ASSISTANT_ITEM = new Set([
  "reasoning",
  "function_call",
  "custom_tool_call",
  "local_shell_call",
]);
const USER_ITEM = new Set([
  "function_call_output",
  "custom_tool_call_output",
  "local_shell_call_output",
]);

export function parseCodexRollout(filePath) {
  const lines = fs.readFileSync(filePath, "utf8").split("\n").filter(Boolean);
  const ir = { source: "codex", cwd: "", model: null, title: null, turns: [] };

  let cur = null; // { role, blocks }
  const flush = () => {
    if (cur && cur.blocks.length) ir.turns.push(cur);
    cur = null;
  };
  const push = (role, block) => {
    if (!cur || cur.role !== role) {
      flush();
      cur = { role, blocks: [] };
    }
    cur.blocks.push(block);
  };

  for (const line of lines) {
    let rec;
    try {
      rec = JSON.parse(line);
    } catch {
      continue; // tolerate partial/garbage lines (version drift)
    }
    if (rec.type === "session_meta") {
      const p = rec.payload || {};
      ir.cwd = p.cwd || ir.cwd;
      ir.model = p.model || ir.model;
      continue;
    }
    if (rec.type !== "response_item") continue;
    const p = rec.payload || {};
    const ptype = p.type;

    if (ptype === "message") {
      const role = p.role;
      if (role === "user") push("user", { kind: "text", text: asText(p.content) });
      else if (role === "assistant")
        push("assistant", { kind: "text", text: asText(p.content) });
      // developer / system / tool messages are session scaffolding — skip,
      // so the imported session starts clean with the first real user turn.
      continue;
    }
    if (ptype === "reasoning") {
      const text = asText(p.summary) || asText(p.content);
      if (text) push("assistant", { kind: "reasoning", text });
      continue;
    }
    if (ptype === "function_call") {
      push("assistant", {
        kind: "tool_use",
        id: p.call_id,
        name: p.name || "tool",
        input: tryParseJSON(p.arguments),
      });
      continue;
    }
    if (ptype === "custom_tool_call") {
      push("assistant", {
        kind: "tool_use",
        id: p.call_id,
        name: p.name || "tool",
        input: tryParseJSON(p.input),
      });
      continue;
    }
    if (USER_ITEM.has(ptype)) {
      push("user", {
        kind: "tool_result",
        forId: p.call_id,
        output: typeof p.output === "string" ? p.output : JSON.stringify(p.output ?? ""),
        isError: false,
      });
      continue;
    }
  }
  flush();
  return ir;
}

// ---------------------------------------------------------------------------
// Claude session  ->  IR
// ---------------------------------------------------------------------------

export function parseClaudeSession(filePath) {
  const lines = fs.readFileSync(filePath, "utf8").split("\n").filter(Boolean);
  const ir = { source: "claude", cwd: "", model: null, title: null, turns: [] };

  for (const line of lines) {
    let rec;
    try {
      rec = JSON.parse(line);
    } catch {
      continue;
    }
    if (rec.cwd && !ir.cwd) ir.cwd = rec.cwd;
    if (rec.type !== "user" && rec.type !== "assistant") continue;
    const msg = rec.message || {};
    if (msg.model && !ir.model) ir.model = msg.model;

    const blocks = [];
    const content = msg.content;
    if (typeof content === "string") {
      blocks.push({ kind: "text", text: content });
    } else if (Array.isArray(content)) {
      for (const b of content) {
        if (!b || typeof b !== "object") continue;
        if (b.type === "text") blocks.push({ kind: "text", text: b.text || "" });
        else if (b.type === "thinking")
          blocks.push({ kind: "reasoning", text: b.thinking || "" });
        else if (b.type === "tool_use")
          blocks.push({ kind: "tool_use", id: b.id, name: b.name, input: b.input });
        else if (b.type === "tool_result")
          blocks.push({
            kind: "tool_result",
            forId: b.tool_use_id,
            output: asText(b.content),
            isError: !!b.is_error,
          });
      }
    }
    if (blocks.length) ir.turns.push({ role: rec.type, blocks });
  }
  return ir;
}

// ---------------------------------------------------------------------------
// IR  ->  Claude session (.jsonl under ~/.claude/projects/<slug>/)
// ---------------------------------------------------------------------------

export function emitClaudeSession(ir, opts = {}) {
  const claudeHome = opts.claudeHome || DEFAULT_CLAUDE_HOME;
  const cwd = ir.cwd || opts.cwd || HOME;
  const model = ir.model || opts.model || "claude-sonnet-4-6";
  const version = opts.version || "2.1.149";
  const sessionId = randomUUID();

  const dir = path.join(claudeHome, "projects", claudeSlug(cwd));
  fs.mkdirSync(dir, { recursive: true });
  const outPath = path.join(dir, `${sessionId}.jsonl`);

  const toolu = new Map(); // source call id -> minted toolu_ id
  const idFor = (srcId) => {
    if (!srcId) return newTooluId();
    if (!toolu.has(srcId)) toolu.set(srcId, newTooluId());
    return toolu.get(srcId);
  };

  let parentUuid = null;
  let t = Date.now();
  const out = [];

  for (const turn of ir.turns) {
    const uuid = randomUUID();
    const content = [];
    for (const b of turn.blocks) {
      if (b.kind === "text") {
        if (b.text) content.push({ type: "text", text: b.text });
      } else if (b.kind === "reasoning") {
        // Emit as text (NOT a thinking block): thinking requires an
        // Anthropic signature we can't forge, and an unsigned thinking
        // block breaks on resume. A labeled text block is safe + readable.
        if (b.text) content.push({ type: "text", text: `🧠 (reasoning)\n${b.text}` });
      } else if (b.kind === "tool_use") {
        content.push({
          type: "tool_use",
          id: idFor(b.id),
          name: b.name || "tool",
          input: b.input ?? {},
        });
      } else if (b.kind === "tool_result") {
        content.push({
          type: "tool_result",
          tool_use_id: idFor(b.forId),
          content: b.output || "",
          ...(b.isError ? { is_error: true } : {}),
        });
      }
    }
    if (!content.length) continue;

    const message =
      turn.role === "assistant"
        ? { role: "assistant", model, content }
        : { role: "user", content };

    out.push({
      parentUuid,
      isSidechain: false,
      userType: "external",
      cwd,
      sessionId,
      version,
      gitBranch: "",
      type: turn.role,
      message,
      uuid,
      timestamp: new Date(t).toISOString(),
    });
    parentUuid = uuid;
    t += 1000;
  }

  fs.writeFileSync(outPath, out.map((o) => JSON.stringify(o)).join("\n") + "\n");
  return { path: outPath, sessionId, cwd, turns: out.length };
}

// ---------------------------------------------------------------------------
// IR  ->  Codex rollout (best-effort; see header note)
// ---------------------------------------------------------------------------

export function emitCodexRollout(ir, opts = {}) {
  const codexHome = opts.codexHome || DEFAULT_CODEX_HOME;
  const cwd = ir.cwd || opts.cwd || HOME;
  const sessionId = randomUUID();
  const now = new Date();
  const stamp = now.toISOString().replace(/[:.]/g, "-").replace("Z", "");
  const y = String(now.getUTCFullYear());
  const m = String(now.getUTCMonth() + 1).padStart(2, "0");
  const d = String(now.getUTCDate()).padStart(2, "0");

  const dir = path.join(codexHome, "sessions", y, m, d);
  fs.mkdirSync(dir, { recursive: true });
  const outPath = path.join(dir, `rollout-${stamp}-${sessionId}.jsonl`);

  const out = [];
  const line = (type, payload) => out.push({ timestamp: new Date().toISOString(), type, payload });

  line("session_meta", {
    id: sessionId,
    timestamp: now.toISOString(),
    cwd,
    originator: "claudies-import",
    cli_version: opts.cliVersion || "imported",
    source: "claude-code",
    git: null,
  });

  for (const turn of ir.turns) {
    for (const b of turn.blocks) {
      if (b.kind === "text") {
        line("response_item", {
          type: "message",
          role: turn.role,
          content: [
            { type: turn.role === "assistant" ? "output_text" : "input_text", text: b.text || "" },
          ],
        });
      } else if (b.kind === "reasoning") {
        line("response_item", { type: "reasoning", summary: [{ type: "summary_text", text: b.text || "" }] });
      } else if (b.kind === "tool_use") {
        line("response_item", {
          type: "function_call",
          name: b.name || "tool",
          arguments: typeof b.input === "string" ? b.input : JSON.stringify(b.input ?? {}),
          call_id: b.id || `call_${randomBytes(8).toString("hex")}`,
        });
      } else if (b.kind === "tool_result") {
        line("response_item", {
          type: "function_call_output",
          call_id: b.forId || `call_${randomBytes(8).toString("hex")}`,
          output: b.output || "",
        });
      }
    }
  }

  fs.writeFileSync(outPath, out.map((o) => JSON.stringify(o)).join("\n") + "\n");

  // Append a session_index.jsonl entry so `codex resume <id>` can find it.
  // The TUI picker also consults SQLite (state_5.sqlite "threads"), which we
  // intentionally do not write — see header note.
  const idxPath = path.join(codexHome, "session_index.jsonl");
  const title = ir.title || firstUserSnippet(ir) || "Imported from Claude Code";
  try {
    fs.appendFileSync(
      idxPath,
      JSON.stringify({ id: sessionId, thread_name: title, updated_at: now.toISOString() }) + "\n"
    );
  } catch {
    /* index is best-effort */
  }

  return { path: outPath, sessionId, cwd, turns: out.length, indexed: true, picker: "maybe" };
}

function firstUserSnippet(ir) {
  for (const turn of ir.turns) {
    if (turn.role !== "user") continue;
    for (const b of turn.blocks) {
      if (b.kind === "text" && b.text) return b.text.slice(0, 60).replace(/\s+/g, " ").trim();
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Session resolution (find a source session by id, name, or path)
// ---------------------------------------------------------------------------

/** Resolve a Codex session: accept a path, a UUID, or a thread_name substring. */
export function resolveCodexSession(idOrPathOrName, codexHome = DEFAULT_CODEX_HOME) {
  if (idOrPathOrName && fs.existsSync(idOrPathOrName)) return idOrPathOrName;
  const sessionsDir = path.join(codexHome, "sessions");
  if (!fs.existsSync(sessionsDir)) return null;
  const all = [];
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, e.name);
      if (e.isDirectory()) walk(full);
      else if (e.name.startsWith("rollout-") && e.name.endsWith(".jsonl")) all.push(full);
    }
  };
  walk(sessionsDir);
  if (!idOrPathOrName) return all.sort().pop() || null; // most recent
  const byId = all.find((f) => f.includes(idOrPathOrName));
  if (byId) return byId;
  // resolve via session_index thread_name
  const idxPath = path.join(codexHome, "session_index.jsonl");
  if (fs.existsSync(idxPath)) {
    const idx = fs
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
    const hit = idx.find((r) => (r.thread_name || "").includes(idOrPathOrName));
    if (hit) return all.find((f) => f.includes(hit.id)) || null;
  }
  return null;
}

/** Resolve a Claude session: accept a path, or a session-id UUID (search projects/). */
export function resolveClaudeSession(idOrPath, claudeHome = DEFAULT_CLAUDE_HOME) {
  if (idOrPath && fs.existsSync(idOrPath)) return idOrPath;
  const projects = path.join(claudeHome, "projects");
  if (!fs.existsSync(projects)) return null;
  const all = [];
  for (const slug of fs.readdirSync(projects)) {
    const dir = path.join(projects, slug);
    if (!fs.statSync(dir).isDirectory()) continue;
    for (const f of fs.readdirSync(dir)) {
      if (f.endsWith(".jsonl")) all.push(path.join(dir, f));
    }
  }
  if (!idOrPath) return all.sort().pop() || null;
  return all.find((f) => f.includes(idOrPath)) || null;
}

// ---------------------------------------------------------------------------
// High-level
// ---------------------------------------------------------------------------

export function codexToClaude(source, opts = {}) {
  const file = resolveCodexSession(source, opts.codexHome);
  if (!file) throw new Error(`No Codex session found for: ${source ?? "(latest)"}`);
  const ir = parseCodexRollout(file);
  if (!ir.turns.length) throw new Error(`Codex session has no convertible turns: ${file}`);
  const res = emitClaudeSession(ir, opts);
  return { from: file, ...res };
}

export function claudeToCodex(source, opts = {}) {
  const file = resolveClaudeSession(source, opts.claudeHome);
  if (!file) throw new Error(`No Claude session found for: ${source ?? "(latest)"}`);
  const ir = parseClaudeSession(file);
  if (!ir.turns.length) throw new Error(`Claude session has no convertible turns: ${file}`);
  const res = emitCodexRollout(ir, opts);
  return { from: file, ...res };
}

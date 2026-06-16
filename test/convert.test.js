// Tests for cross-tool session conversion (src/convert.js).
//
// These use synthetic fixtures written to a temp dir — no dependency on the
// user's real ~/.claude or ~/.codex. We assert the things that matter for a
// resumable imported session: valid JSONL, an intact parentUuid chain, a
// correct tool_use<->tool_result id bijection (no dangling references), and
// lossless turn-count round-tripping through the intermediate representation.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  parseCodexRollout,
  parseClaudeSession,
  emitClaudeSession,
  emitCodexRollout,
  claudeSlug,
} from "../src/convert.js";

function tmpdir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "claudies-convtest-"));
}

// A small but representative Codex rollout: meta, user message, reasoning,
// a function_call + its output, then an assistant reply.
function codexFixture(dir) {
  const lines = [
    { timestamp: "t0", type: "session_meta", payload: { id: "x", cwd: "/Users/me/proj", model: "gpt-5" } },
    { timestamp: "t1", type: "response_item", payload: { type: "message", role: "developer", content: [{ type: "input_text", text: "system scaffolding" }] } },
    { timestamp: "t2", type: "response_item", payload: { type: "message", role: "user", content: [{ type: "input_text", text: "list the files" }] } },
    { timestamp: "t3", type: "response_item", payload: { type: "reasoning", summary: [{ type: "summary_text", text: "I'll run ls" }], encrypted_content: "SECRET" } },
    { timestamp: "t4", type: "response_item", payload: { type: "function_call", name: "exec_command", arguments: '{"cmd":"ls"}', call_id: "call_abc" } },
    { timestamp: "t5", type: "response_item", payload: { type: "function_call_output", call_id: "call_abc", output: "a.txt\nb.txt" } },
    { timestamp: "t6", type: "response_item", payload: { type: "message", role: "assistant", content: [{ type: "output_text", text: "There are two files." }] } },
  ];
  const f = path.join(dir, "rollout-test.jsonl");
  fs.writeFileSync(f, lines.map((l) => JSON.stringify(l)).join("\n") + "\n");
  return f;
}

test("claudeSlug: / and . both become -", () => {
  assert.equal(claudeSlug("/Users/tao.shen/LLM/.claude/x"), "-Users-tao-shen-LLM--claude-x");
});

test("parseCodexRollout: extracts turns, drops developer scaffolding, keeps tool call+output", () => {
  const dir = tmpdir();
  const ir = parseCodexRollout(codexFixture(dir));
  assert.equal(ir.source, "codex");
  assert.equal(ir.cwd, "/Users/me/proj");
  const kinds = {};
  for (const t of ir.turns) for (const b of t.blocks) kinds[b.kind] = (kinds[b.kind] || 0) + 1;
  assert.equal(kinds.tool_use, 1);
  assert.equal(kinds.tool_result, 1);
  assert.equal(kinds.reasoning, 1);
  // developer message must NOT survive as a turn
  const hasDevText = ir.turns.some((t) => t.blocks.some((b) => b.kind === "text" && b.text.includes("scaffolding")));
  assert.equal(hasDevText, false);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("codex -> claude: valid JSONL, intact parentUuid chain, no dangling tool ids, starts with user", () => {
  const dir = tmpdir();
  const ir = parseCodexRollout(codexFixture(dir));
  const claudeHome = path.join(dir, "claude");
  const res = emitClaudeSession(ir, { claudeHome });

  // file landed under projects/<slug>/<sessionId>.jsonl
  assert.ok(res.path.includes(path.join("projects", claudeSlug(ir.cwd))));

  const lines = fs.readFileSync(res.path, "utf8").split("\n").filter(Boolean);
  assert.ok(lines.length >= 3);

  let prev = null;
  let firstType = null;
  const toolUse = new Set();
  const toolResultRefs = new Set();
  lines.forEach((l, i) => {
    const o = JSON.parse(l); // throws if not valid JSON
    if (i === 0) firstType = o.type;
    assert.equal(o.parentUuid, prev, `line ${i} parentUuid should chain to previous uuid`);
    prev = o.uuid;
    assert.ok(o.sessionId && o.uuid && o.timestamp && o.message);
    for (const b of o.message.content || []) {
      if (b.type === "tool_use") toolUse.add(b.id);
      if (b.type === "tool_result") toolResultRefs.add(b.tool_use_id);
    }
  });

  assert.equal(firstType, "user", "imported session should open with a user turn");
  // every tool_result points at an emitted tool_use (id bijection holds)
  for (const ref of toolResultRefs) assert.ok(toolUse.has(ref), `dangling tool_result ref: ${ref}`);
  // minted ids are in the Claude namespace
  for (const id of toolUse) assert.match(id, /^toolu_/);
  // crypto reasoning payload must not leak through
  assert.ok(!fs.readFileSync(res.path, "utf8").includes("SECRET"));

  fs.rmSync(dir, { recursive: true, force: true });
});

test("round-trip: codex -> claude -> parse preserves turn count", () => {
  const dir = tmpdir();
  const ir = parseCodexRollout(codexFixture(dir));
  const res = emitClaudeSession(ir, { claudeHome: path.join(dir, "claude") });
  const ir2 = parseClaudeSession(res.path);
  assert.equal(ir2.turns.length, ir.turns.length);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("claude -> codex: writes a valid rollout + session_index entry", () => {
  const dir = tmpdir();
  // build a claude session by emitting from a codex IR first (reuse fixture)
  const srcIr = parseCodexRollout(codexFixture(dir));
  const claudeHome = path.join(dir, "claude");
  const claudeSession = emitClaudeSession(srcIr, { claudeHome });

  const ir = parseClaudeSession(claudeSession.path);
  const codexHome = path.join(dir, "codex");
  const res = emitCodexRollout(ir, { codexHome });

  const lines = fs.readFileSync(res.path, "utf8").split("\n").filter(Boolean);
  const recs = lines.map((l) => JSON.parse(l)); // valid JSONL
  assert.equal(recs[0].type, "session_meta");
  assert.ok(recs.some((r) => r.type === "response_item" && r.payload.type === "function_call"));
  assert.ok(recs.some((r) => r.type === "response_item" && r.payload.type === "function_call_output"));

  // session_index.jsonl got an entry pointing at this session
  const idx = fs.readFileSync(path.join(codexHome, "session_index.jsonl"), "utf8");
  assert.ok(idx.includes(res.sessionId));

  fs.rmSync(dir, { recursive: true, force: true });
});

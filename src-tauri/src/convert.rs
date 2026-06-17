//! Cross-tool session conversion — a faithful port of `src/convert.js` so the
//! GUI can import a session from one tool into the other as a brand-new,
//! resumable session (the CLI keeps using the JS version).
//!
//! NOT a lossless sync: the two on-disk formats agree on the easy middle
//! (text turns, tool calls) but diverge at the hard edges (Claude's parentUuid
//! DAG vs Codex's linear log; provider-private reasoning). We convert through a
//! lossy intermediate representation, preserving two deliberate concessions:
//!   * reasoning → a labeled TEXT block (an unsigned `thinking` block would
//!     break Claude resume), and
//!   * tool-call ids are RE-MINTED in the target namespace via a bijection.
//!
//! codex → claude is CLEAN (we only write a new ~/.claude/projects/*.jsonl,
//! which Claude indexes with no DB surgery). claude → codex is BEST-EFFORT.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use serde::Serialize;
use serde_json::{json, Value};

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

fn default_claude_home() -> PathBuf {
    home().join(".claude")
}

fn default_codex_home() -> PathBuf {
    match std::env::var("CODEX_HOME") {
        Ok(h) if !h.is_empty() => PathBuf::from(h),
        _ => home().join(".codex"),
    }
}

/// macOS/Claude project-dir slug: every "/" and "." in the cwd becomes "-".
fn claude_slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

fn new_toolu_id() -> String {
    let u = uuid::Uuid::new_v4().simple().to_string();
    format!("toolu_{}", &u[..24])
}

fn try_parse_json(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| json!({ "raw": s }))
}

/// Codex message content / Claude content blocks → joined plain text.
fn as_text(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|p| {
                if let Some(s) = p.as_str() {
                    Some(s.to_string())
                } else {
                    p.get("text").and_then(|t| t.as_str()).map(String::from)
                }
            })
            .collect::<Vec<_>>()
            .join("");
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Intermediate representation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Block {
    Text(String),
    Reasoning(String),
    ToolUse { id: Option<String>, name: String, input: Value },
    ToolResult { for_id: Option<String>, output: String, is_error: bool },
}

#[derive(Debug, Clone)]
struct Turn {
    role: String, // "user" | "assistant"
    blocks: Vec<Block>,
}

#[derive(Debug, Default)]
struct Ir {
    cwd: String,
    model: Option<String>,
    turns: Vec<Turn>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub from: String,
    pub path: String,
    pub session_id: String,
    pub cwd: String,
    pub turns: usize,
    /// "maybe" for claude→codex (may not show in the TUI picker until reindex).
    pub picker: Option<String>,
}

struct TurnAccumulator {
    cur: Option<Turn>,
    turns: Vec<Turn>,
}
impl TurnAccumulator {
    fn new() -> Self {
        Self { cur: None, turns: Vec::new() }
    }
    fn flush(&mut self) {
        if let Some(t) = self.cur.take() {
            if !t.blocks.is_empty() {
                self.turns.push(t);
            }
        }
    }
    fn push(&mut self, role: &str, block: Block) {
        let same = self.cur.as_ref().map(|t| t.role == role).unwrap_or(false);
        if !same {
            self.flush();
            self.cur = Some(Turn { role: role.to_string(), blocks: Vec::new() });
        }
        self.cur.as_mut().unwrap().blocks.push(block);
    }
}

// ---------------------------------------------------------------------------
// Codex rollout → IR
// ---------------------------------------------------------------------------

const USER_ITEMS: &[&str] = &[
    "function_call_output",
    "custom_tool_call_output",
    "local_shell_call_output",
];

fn parse_codex_rollout(path: &Path) -> Ir {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let mut ir = Ir::default();
    let mut acc = TurnAccumulator::new();

    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let rec: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let rtype = rec.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if rtype == "session_meta" {
            let p = rec.get("payload").cloned().unwrap_or(json!({}));
            if let Some(c) = p.get("cwd").and_then(|v| v.as_str()) {
                ir.cwd = c.to_string();
            }
            if let Some(m) = p.get("model").and_then(|v| v.as_str()) {
                ir.model = Some(m.to_string());
            }
            continue;
        }
        if rtype != "response_item" {
            continue;
        }
        let p = match rec.get("payload") {
            Some(p) => p,
            None => continue,
        };
        let ptype = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let empty = json!(null);
        match ptype {
            "message" => {
                let role = p.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let content = p.get("content").unwrap_or(&empty);
                if role == "user" {
                    acc.push("user", Block::Text(as_text(content)));
                } else if role == "assistant" {
                    acc.push("assistant", Block::Text(as_text(content)));
                }
                // developer/system/tool messages are scaffolding — skip.
            }
            "reasoning" => {
                let text = {
                    let s = as_text(p.get("summary").unwrap_or(&empty));
                    if s.is_empty() { as_text(p.get("content").unwrap_or(&empty)) } else { s }
                };
                if !text.is_empty() {
                    acc.push("assistant", Block::Reasoning(text));
                }
            }
            "function_call" => {
                let args = p.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                acc.push(
                    "assistant",
                    Block::ToolUse {
                        id: p.get("call_id").and_then(|v| v.as_str()).map(String::from),
                        name: p.get("name").and_then(|v| v.as_str()).unwrap_or("tool").to_string(),
                        input: try_parse_json(args),
                    },
                );
            }
            "custom_tool_call" => {
                let inp = p.get("input").and_then(|v| v.as_str()).unwrap_or("");
                acc.push(
                    "assistant",
                    Block::ToolUse {
                        id: p.get("call_id").and_then(|v| v.as_str()).map(String::from),
                        name: p.get("name").and_then(|v| v.as_str()).unwrap_or("tool").to_string(),
                        input: try_parse_json(inp),
                    },
                );
            }
            t if USER_ITEMS.contains(&t) => {
                let output = match p.get("output") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => String::new(),
                };
                acc.push(
                    "user",
                    Block::ToolResult {
                        for_id: p.get("call_id").and_then(|v| v.as_str()).map(String::from),
                        output,
                        is_error: false,
                    },
                );
            }
            _ => {}
        }
    }
    acc.flush();
    ir.turns = acc.turns;
    ir
}

// ---------------------------------------------------------------------------
// Claude session → IR
// ---------------------------------------------------------------------------

fn parse_claude_session(path: &Path) -> Ir {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let mut ir = Ir::default();

    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let rec: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if ir.cwd.is_empty() {
            if let Some(c) = rec.get("cwd").and_then(|v| v.as_str()) {
                ir.cwd = c.to_string();
            }
        }
        let rtype = rec.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if rtype != "user" && rtype != "assistant" {
            continue;
        }
        let msg = rec.get("message").cloned().unwrap_or(json!({}));
        if ir.model.is_none() {
            if let Some(m) = msg.get("model").and_then(|v| v.as_str()) {
                ir.model = Some(m.to_string());
            }
        }
        let mut blocks = Vec::new();
        match msg.get("content") {
            Some(Value::String(s)) => blocks.push(Block::Text(s.clone())),
            Some(Value::Array(arr)) => {
                for b in arr {
                    let bt = b.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match bt {
                        "text" => blocks.push(Block::Text(
                            b.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        )),
                        "thinking" => blocks.push(Block::Reasoning(
                            b.get("thinking").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        )),
                        "tool_use" => blocks.push(Block::ToolUse {
                            id: b.get("id").and_then(|v| v.as_str()).map(String::from),
                            name: b.get("name").and_then(|v| v.as_str()).unwrap_or("tool").to_string(),
                            input: b.get("input").cloned().unwrap_or(json!({})),
                        }),
                        "tool_result" => blocks.push(Block::ToolResult {
                            for_id: b.get("tool_use_id").and_then(|v| v.as_str()).map(String::from),
                            output: as_text(b.get("content").unwrap_or(&json!(null))),
                            is_error: b.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false),
                        }),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if !blocks.is_empty() {
            ir.turns.push(Turn { role: rtype.to_string(), blocks });
        }
    }
    ir
}

// ---------------------------------------------------------------------------
// IR → Claude session (clean: a new ~/.claude/projects/<slug>/<uuid>.jsonl)
// ---------------------------------------------------------------------------

fn emit_claude_session(ir: &Ir, claude_home: &Path) -> Result<(String, String, usize), String> {
    use std::collections::HashMap;
    let cwd = if ir.cwd.is_empty() { home().to_string_lossy().to_string() } else { ir.cwd.clone() };
    let model = ir.model.clone().unwrap_or_else(|| "claude-sonnet-4-6".to_string());
    let session_id = uuid::Uuid::new_v4().to_string();

    let dir = claude_home.join("projects").join(claude_slug(&cwd));
    fs::create_dir_all(&dir).map_err(|e| format!("Create {}: {e}", dir.display()))?;
    let out_path = dir.join(format!("{session_id}.jsonl"));

    let mut toolu: HashMap<String, String> = HashMap::new();
    let mut id_for = |src: &Option<String>| -> String {
        match src {
            None => new_toolu_id(),
            Some(s) => toolu.entry(s.clone()).or_insert_with(new_toolu_id).clone(),
        }
    };

    let mut parent: Value = Value::Null;
    let base_ms: i64 = Utc::now().timestamp_millis();
    let mut lines: Vec<String> = Vec::new();
    let mut t = base_ms;

    for turn in &ir.turns {
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut content: Vec<Value> = Vec::new();
        for b in &turn.blocks {
            match b {
                Block::Text(text) => {
                    if !text.is_empty() {
                        content.push(json!({ "type": "text", "text": text }));
                    }
                }
                Block::Reasoning(text) => {
                    if !text.is_empty() {
                        content.push(json!({ "type": "text", "text": format!("🧠 (reasoning)\n{text}") }));
                    }
                }
                Block::ToolUse { id, name, input } => {
                    content.push(json!({
                        "type": "tool_use",
                        "id": id_for(id),
                        "name": name,
                        "input": input,
                    }));
                }
                Block::ToolResult { for_id, output, is_error } => {
                    let mut cell = json!({
                        "type": "tool_result",
                        "tool_use_id": id_for(for_id),
                        "content": output,
                    });
                    if *is_error {
                        cell["is_error"] = json!(true);
                    }
                    content.push(cell);
                }
            }
        }
        if content.is_empty() {
            continue;
        }
        let message = if turn.role == "assistant" {
            json!({ "role": "assistant", "model": model, "content": content })
        } else {
            json!({ "role": "user", "content": content })
        };
        let ts = Utc.timestamp_millis_opt(t).single().unwrap_or_else(Utc::now);
        let rec = json!({
            "parentUuid": parent,
            "isSidechain": false,
            "userType": "external",
            "cwd": cwd,
            "sessionId": session_id,
            "version": "2.1.149",
            "gitBranch": "",
            "type": turn.role,
            "message": message,
            "uuid": uuid,
            "timestamp": ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        });
        lines.push(rec.to_string());
        parent = json!(uuid);
        t += 1000;
    }

    let written = lines.len();
    fs::write(&out_path, lines.join("\n") + "\n")
        .map_err(|e| format!("Write {}: {e}", out_path.display()))?;
    Ok((out_path.to_string_lossy().to_string(), session_id, written))
}

// ---------------------------------------------------------------------------
// IR → Codex rollout (best-effort)
// ---------------------------------------------------------------------------

fn emit_codex_rollout(ir: &Ir, codex_home: &Path) -> Result<(String, String, usize), String> {
    let cwd = if ir.cwd.is_empty() { home().to_string_lossy().to_string() } else { ir.cwd.clone() };
    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let stamp = now.format("%Y-%m-%dT%H-%M-%S").to_string();
    let dir = codex_home
        .join("sessions")
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string());
    fs::create_dir_all(&dir).map_err(|e| format!("Create {}: {e}", dir.display()))?;
    let out_path = dir.join(format!("rollout-{stamp}-{session_id}.jsonl"));

    let mut lines: Vec<String> = Vec::new();
    let mut line = |ty: &str, payload: Value| {
        lines.push(
            json!({ "timestamp": Utc::now().to_rfc3339(), "type": ty, "payload": payload }).to_string(),
        );
    };

    line(
        "session_meta",
        json!({
            "id": session_id,
            "timestamp": now.to_rfc3339(),
            "cwd": cwd,
            "originator": "claudex-import",
            "cli_version": "imported",
            "source": "claude-code",
            "git": Value::Null,
        }),
    );

    for turn in &ir.turns {
        for b in &turn.blocks {
            match b {
                Block::Text(text) => line(
                    "response_item",
                    json!({
                        "type": "message",
                        "role": turn.role,
                        "content": [{
                            "type": if turn.role == "assistant" { "output_text" } else { "input_text" },
                            "text": text,
                        }],
                    }),
                ),
                Block::Reasoning(text) => line(
                    "response_item",
                    json!({ "type": "reasoning", "summary": [{ "type": "summary_text", "text": text }] }),
                ),
                Block::ToolUse { id, name, input } => {
                    let arguments = match input {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    line(
                        "response_item",
                        json!({
                            "type": "function_call",
                            "name": name,
                            "arguments": arguments,
                            "call_id": id.clone().unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple())),
                        }),
                    );
                }
                Block::ToolResult { for_id, output, .. } => line(
                    "response_item",
                    json!({
                        "type": "function_call_output",
                        "call_id": for_id.clone().unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple())),
                        "output": output,
                    }),
                ),
            }
        }
    }

    let written = lines.len();
    fs::write(&out_path, lines.join("\n") + "\n")
        .map_err(|e| format!("Write {}: {e}", out_path.display()))?;

    // Append a session_index.jsonl entry (best-effort; the TUI picker also
    // reads SQLite, which we deliberately don't touch).
    let title = first_user_snippet(ir).unwrap_or_else(|| "Imported from Claude Code".to_string());
    let idx = codex_home.join("session_index.jsonl");
    if let Ok(mut existing) = fs::read_to_string(&idx).or_else(|_| Ok::<String, std::io::Error>(String::new())) {
        let entry = json!({ "id": session_id, "thread_name": title, "updated_at": now.to_rfc3339() }).to_string();
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str(&entry);
        existing.push('\n');
        let _ = fs::write(&idx, existing);
    }

    Ok((out_path.to_string_lossy().to_string(), session_id, written))
}

fn first_user_snippet(ir: &Ir) -> Option<String> {
    for turn in &ir.turns {
        if turn.role != "user" {
            continue;
        }
        for b in &turn.blocks {
            if let Block::Text(t) = b {
                if !t.is_empty() {
                    let s: String = t.chars().take(60).collect();
                    return Some(s.split_whitespace().collect::<Vec<_>>().join(" "));
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Session resolution
// ---------------------------------------------------------------------------

pub(crate) fn walk_rollouts(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk_rollouts(&p, out);
            } else if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
                if n.starts_with("rollout-") && n.ends_with(".jsonl") {
                    out.push(p);
                }
            }
        }
    }
}

/// Cheap head-only read of a Codex rollout: parse just the FIRST line
/// (`session_meta`) for (id, cwd, model). Used by list views to group sessions
/// by project without parsing the whole transcript. Falls back to the trailing
/// UUID in the filename for the id. Returns None if the file can't be read.
pub(crate) fn read_rollout_meta(path: &Path) -> Option<(String, String, Option<String>)> {
    let file = fs::File::open(path).ok()?;
    let mut first = String::new();
    {
        use std::io::BufRead;
        std::io::BufReader::new(file).read_line(&mut first).ok()?;
    }
    let rec: Value = serde_json::from_str(first.trim()).ok()?;
    let payload = rec.get("payload").cloned().unwrap_or(json!({}));
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| id_from_rollout_filename(path));
    let cwd = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model = payload
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some((id, cwd, model))
}

/// Extract the trailing UUID from `rollout-<ts>-<uuid>.jsonl`, or the stem.
fn id_from_rollout_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    // UUID is the last 5 dash-groups (8-4-4-4-12).
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 5 {
        parts[parts.len() - 5..].join("-")
    } else {
        stem
    }
}

/// Resolve a session's on-disk file by id within a home (codex rollout or claude
/// jsonl). Returns the path as a String for the GUI's view/delete actions.
pub fn resolve_session_file(needle: &str, home: &Path, world: &str) -> Option<String> {
    let file = if world == "codex" {
        resolve_codex_session(needle, home)
    } else {
        resolve_claude_session(needle, home)
    }?;
    Some(file.to_string_lossy().to_string())
}

/// Render a session (by id, within a home) to readable markdown for the viewer.
pub fn session_transcript(needle: &str, home: &Path, world: &str) -> Result<String, String> {
    let file = if world == "codex" {
        resolve_codex_session(needle, home)
    } else {
        resolve_claude_session(needle, home)
    }
    .ok_or_else(|| format!("Session not found: {needle}"))?;
    let ir = if world == "codex" {
        parse_codex_rollout(&file)
    } else {
        parse_claude_session(&file)
    };
    Ok(render_ir_text(&ir))
}

/// Walk the IR into a compact, readable markdown transcript (read-only view).
fn render_ir_text(ir: &Ir) -> String {
    let mut out = String::new();
    if !ir.cwd.is_empty() {
        out.push_str(&format!("**cwd** `{}`", ir.cwd));
    }
    if let Some(m) = &ir.model {
        out.push_str(&format!("  ·  **model** `{m}`"));
    }
    out.push_str("\n\n---\n\n");
    for turn in &ir.turns {
        let who = if turn.role == "user" { "🧑 User" } else { "🤖 Assistant" };
        out.push_str(&format!("### {who}\n\n"));
        for b in &turn.blocks {
            match b {
                Block::Text(t) => {
                    out.push_str(t.trim());
                    out.push_str("\n\n");
                }
                Block::Reasoning(t) => {
                    out.push_str("> 🧠 ");
                    out.push_str(t.trim().replace('\n', "\n> ").as_str());
                    out.push_str("\n\n");
                }
                Block::ToolUse { name, input, .. } => {
                    let arg = serde_json::to_string(input).unwrap_or_default();
                    // Char-safe truncation — byte slicing panics mid-UTF-8 (CJK/emoji).
                    let arg = if arg.chars().count() > 200 {
                        format!("{}…", arg.chars().take(200).collect::<String>())
                    } else {
                        arg
                    };
                    out.push_str(&format!("→ **{name}**(`{arg}`)\n\n"));
                }
                Block::ToolResult { output, is_error, .. } => {
                    let o = if output.chars().count() > 400 {
                        format!("{}…", output.chars().take(400).collect::<String>())
                    } else {
                        output.clone()
                    };
                    out.push_str(&format!(
                        "← {}\n```\n{}\n```\n\n",
                        if *is_error { "error" } else { "result" },
                        o.trim()
                    ));
                }
            }
        }
    }
    out
}

fn resolve_codex_session(needle: &str, codex_home: &Path) -> Option<PathBuf> {
    if !needle.is_empty() && Path::new(needle).exists() {
        return Some(PathBuf::from(needle));
    }
    let sessions = codex_home.join("sessions");
    if !sessions.exists() {
        return None;
    }
    let mut all = Vec::new();
    walk_rollouts(&sessions, &mut all);
    all.sort();
    if needle.is_empty() {
        return all.into_iter().last();
    }
    if let Some(hit) = all.iter().find(|f| f.to_string_lossy().contains(needle)) {
        return Some(hit.clone());
    }
    // resolve via session_index thread_name
    let idx = codex_home.join("session_index.jsonl");
    if let Ok(raw) = fs::read_to_string(&idx) {
        for line in raw.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                let name = v.get("thread_name").and_then(|x| x.as_str()).unwrap_or("");
                if name.contains(needle) {
                    if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                        if let Some(f) = all.iter().find(|f| f.to_string_lossy().contains(id)) {
                            return Some(f.clone());
                        }
                    }
                }
            }
        }
    }
    None
}

fn resolve_claude_session(needle: &str, claude_home: &Path) -> Option<PathBuf> {
    if !needle.is_empty() && Path::new(needle).exists() {
        return Some(PathBuf::from(needle));
    }
    let projects = claude_home.join("projects");
    if !projects.exists() {
        return None;
    }
    let mut all = Vec::new();
    if let Ok(rd) = fs::read_dir(&projects) {
        for slug in rd.flatten() {
            let d = slug.path();
            if !d.is_dir() {
                continue;
            }
            if let Ok(files) = fs::read_dir(&d) {
                for f in files.flatten() {
                    let p = f.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        all.push(p);
                    }
                }
            }
        }
    }
    all.sort();
    if needle.is_empty() {
        return all.into_iter().last();
    }
    all.into_iter().find(|f| f.to_string_lossy().contains(needle))
}

// ---------------------------------------------------------------------------
// High-level — the two Tauri-facing entry points
// ---------------------------------------------------------------------------

/// Import a Codex session that lives in a SPECIFIC codex_home (per-profile).
pub fn import_codex_session_to_claude_in(
    source: &str,
    codex_home: &Path,
) -> Result<ImportResult, String> {
    let file = resolve_codex_session(source, codex_home)
        .ok_or_else(|| format!("No Codex session found for: {}", if source.is_empty() { "(latest)" } else { source }))?;
    let ir = parse_codex_rollout(&file);
    if ir.turns.is_empty() {
        return Err(format!("Codex session has no convertible turns: {}", file.display()));
    }
    let (path, session_id, turns) = emit_claude_session(&ir, &default_claude_home())?;
    let cwd = if ir.cwd.is_empty() { home().to_string_lossy().to_string() } else { ir.cwd };
    Ok(ImportResult { from: file.to_string_lossy().to_string(), path, session_id, cwd, turns, picker: None })
}

/// Convenience: import from the default ~/.codex. The GUI routes through
/// `lib::import_codex_session_to_claude_any_home` (searches every profile home);
/// this single-home entry is kept for symmetry with the reverse direction.
#[allow(dead_code)]
pub fn import_codex_session_to_claude(source: String) -> Result<ImportResult, String> {
    import_codex_session_to_claude_in(&source, &default_codex_home())
}

/// Pick the newest Claude Code CLI transcript (`~/.claude/projects/<slug>/*.jsonl`)
/// for a project cwd. The GUI's "Export to Codex" passes a project cwd, but the
/// converter only understands CLI transcripts (not the Desktop panel store), so
/// we map cwd -> slug -> latest transcript.
fn latest_claude_transcript_for_cwd(cwd: &str, claude_home: &Path) -> Option<PathBuf> {
    let dir = claude_home.join("projects").join(claude_slug(cwd));
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for f in rd.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(p);
            }
        }
    }
    // Transcript filenames are random UUIDs, so sort by mtime (most recent last).
    files.sort_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    files.into_iter().last()
}

fn emit_codex_from_claude_file(file: &Path) -> Result<ImportResult, String> {
    let ir = parse_claude_session(file);
    if ir.turns.is_empty() {
        return Err(format!("Claude session has no convertible turns: {}", file.display()));
    }
    let (path, session_id, turns) = emit_codex_rollout(&ir, &default_codex_home())?;
    let cwd = if ir.cwd.is_empty() { home().to_string_lossy().to_string() } else { ir.cwd };
    Ok(ImportResult {
        from: file.to_string_lossy().to_string(),
        path,
        session_id,
        cwd,
        turns,
        picker: Some("maybe".to_string()),
    })
}

pub fn import_claude_session_to_codex(source: String) -> Result<ImportResult, String> {
    // The GUI passes a project cwd — resolve the newest CLI transcript for it
    // first. Fall back to treating `source` as a session id / explicit path.
    if !source.is_empty() {
        if let Some(file) = latest_claude_transcript_for_cwd(&source, &default_claude_home()) {
            return emit_codex_from_claude_file(&file);
        }
        // A project cwd with no CLI transcript: surface the helpful error
        // directly rather than falling through to resolve_claude_session, which
        // would treat the existing directory path as a (broken) "session".
        if source.contains('/') || Path::new(&source).is_dir() {
            return Err(format!(
                "No Claude Code session found for project \"{source}\". Open it in `claude` first so it has a transcript to export."
            ));
        }
    }
    let file = resolve_claude_session(&source, &default_claude_home()).ok_or_else(|| {
        if source.is_empty() {
            "No Claude Code session found.".to_string()
        } else {
            format!(
                "No Claude Code session found for project \"{source}\". Open it in `claude` first so it has a transcript to export."
            )
        }
    })?;
    emit_codex_from_claude_file(&file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex_fixture(dir: &Path) -> PathBuf {
        let lines = [
            json!({"timestamp":"t0","type":"session_meta","payload":{"id":"x","cwd":"/Users/me/proj","model":"gpt-5"}}),
            json!({"timestamp":"t2","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"list the files"}]}}),
            json!({"timestamp":"t3","type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"I'll run ls"}],"encrypted_content":"SECRET"}}),
            json!({"timestamp":"t4","type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"ls\"}","call_id":"call_abc"}}),
            json!({"timestamp":"t5","type":"response_item","payload":{"type":"function_call_output","call_id":"call_abc","output":"a.txt\nb.txt"}}),
            json!({"timestamp":"t6","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Two files."}]}}),
        ];
        let f = dir.join("rollout-test.jsonl");
        fs::write(&f, lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
        f
    }

    #[test]
    fn read_rollout_meta_head_only_extracts_id_cwd_model() {
        let dir = tempfile::tempdir().unwrap();
        let f = codex_fixture(dir.path());
        let (id, cwd, model) = read_rollout_meta(&f).expect("meta");
        assert_eq!(id, "x");
        assert_eq!(cwd, "/Users/me/proj");
        assert_eq!(model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn latest_claude_transcript_for_cwd_resolves_by_slug() {
        let home = tempfile::tempdir().unwrap();
        let cwd = "/Users/me/proj";
        let dir = home.path().join("projects").join(claude_slug(cwd));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("aaaa.jsonl"), "{}").unwrap();
        fs::write(dir.join("zzzz.jsonl"), "{}").unwrap();
        // Resolves a transcript for the project (mtime-newest; both are valid).
        let got = latest_claude_transcript_for_cwd(cwd, home.path()).expect("transcript");
        assert!(got.extension().and_then(|e| e.to_str()) == Some("jsonl"));
        // Unknown project resolves to nothing.
        assert!(latest_claude_transcript_for_cwd("/no/such/proj", home.path()).is_none());
    }

    #[test]
    fn id_from_rollout_filename_pulls_trailing_uuid() {
        let p = Path::new(
            "/x/rollout-2026-03-27T19-12-50-019d30b6-a900-7100-84eb-12f38d7d8658.jsonl",
        );
        assert_eq!(
            id_from_rollout_filename(p),
            "019d30b6-a900-7100-84eb-12f38d7d8658"
        );
    }

    #[test]
    fn codex_to_claude_clean_jsonl_chain_and_id_bijection() {
        let dir = tempfile::tempdir().unwrap();
        let src = codex_fixture(dir.path());
        let ir = parse_codex_rollout(&src);
        assert_eq!(ir.cwd, "/Users/me/proj");
        let claude_home = dir.path().join("claude");
        let (path, _sid, turns) = emit_claude_session(&ir, &claude_home).unwrap();
        assert!(turns >= 3);
        let raw = fs::read_to_string(&path).unwrap();
        // crypto reasoning never leaks
        assert!(!raw.contains("SECRET"));
        let mut prev: Value = Value::Null;
        let mut tool_use = std::collections::HashSet::new();
        let mut tool_res = std::collections::HashSet::new();
        let mut first_type = String::new();
        for (i, l) in raw.lines().filter(|l| !l.is_empty()).enumerate() {
            let o: Value = serde_json::from_str(l).unwrap();
            if i == 0 {
                first_type = o["type"].as_str().unwrap().to_string();
            }
            assert_eq!(o["parentUuid"], prev, "parentUuid chain");
            prev = o["uuid"].clone();
            if let Some(content) = o["message"]["content"].as_array() {
                for b in content {
                    if b["type"] == "tool_use" {
                        tool_use.insert(b["id"].as_str().unwrap().to_string());
                    }
                    if b["type"] == "tool_result" {
                        tool_res.insert(b["tool_use_id"].as_str().unwrap().to_string());
                    }
                }
            }
        }
        assert_eq!(first_type, "user");
        for r in &tool_res {
            assert!(tool_use.contains(r), "dangling tool_result id");
        }
        for id in &tool_use {
            assert!(id.starts_with("toolu_"));
        }
    }

    #[test]
    fn claude_slug_maps_slash_and_dot() {
        assert_eq!(claude_slug("/Users/tao.shen/LLM/.claude/x"), "-Users-tao-shen-LLM--claude-x");
    }
}

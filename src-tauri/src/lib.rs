mod convert;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const EXT_DIR_NAME: &str = "Claude Extensions";
const EXT_SETTINGS_DIR_NAME: &str = "Claude Extensions Settings";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionEntry {
    pub id: String,
    pub has_settings: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopInstall {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub data_dir: String,
    pub app_path: Option<String>,
    pub launcher_path: Option<String>,
    pub managed: bool,
    /// True when a Claude.app process is currently open against this
    /// data_dir. Detected by parsing `--user-data-dir=` from `ps` output.
    /// "Default" can be `kind == "default"` AND `is_running == false` —
    /// the labels are orthogonal.
    #[serde(default)]
    pub is_running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionSelectionRow {
    pub id: String,
    pub has_settings: bool,
    pub exists_in_target: bool,
    pub target_has_settings: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CopySummary {
    pub copied: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionLibrarySource {
    pub install_id: String,
    pub install_name: String,
    pub data_dir: String,
    pub has_settings: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionTargetStatus {
    pub install_id: String,
    pub install_name: String,
    pub data_dir: String,
    pub kind: String,
    pub has_extension: bool,
    pub has_settings: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionShareItem {
    pub id: String,
    pub sources: Vec<ExtensionLibrarySource>,
    pub targets: Vec<ExtensionTargetStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairExtensionShare {
    pub id: String,
    pub source_has_extension: bool,
    pub target_has_extension: bool,
    pub source_has_settings: bool,
    pub target_has_settings: bool,
    pub shared: bool,
    pub partial: bool,
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PairShareChange {
    pub extension_id: String,
    pub shared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryFile {
    pub version: u32,
    pub profiles: Vec<RegistryProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryProfile {
    pub name: String,
    #[serde(rename = "type")]
    pub profile_type: String,
    pub desktop: Option<RegistryDesktop>,
    pub code: Option<serde_json::Value>,
    /// Codex profile half: { configDir, aliasName, shell }. Optional + default
    /// so registries written before Codex support still deserialize.
    #[serde(default)]
    pub codex: Option<serde_json::Value>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryDesktop {
    #[serde(rename = "dataDir")]
    pub data_dir: String,
    #[serde(rename = "appPath")]
    pub app_path: String,
    #[serde(rename = "claudeAppPath")]
    pub claude_app_path: String,
}

pub fn sanitize_profile_name(name: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;

    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }

    out.trim_matches('-').to_string()
}

fn title_case(name: &str) -> String {
    if name.len() <= 4 {
        return name.to_uppercase();
    }

    name.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())
}

fn config_home() -> Result<PathBuf, String> {
    Ok(env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".config")))
}

fn registry_path() -> Result<PathBuf, String> {
    Ok(config_home()?.join("claude-multiprofile").join("profiles.json"))
}

fn default_desktop_data_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join("Library").join("Application Support").join("Claude"))
}

fn default_data_dir_for(name: &str) -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Library")
        .join("Application Support")
        .join(format!("Claude-{}", title_case(name))))
}

fn default_app_path_for(name: &str) -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Applications")
        .join(format!("Claude {}.app", title_case(name))))
}

fn empty_registry() -> RegistryFile {
    RegistryFile {
        version: 1,
        profiles: Vec::new(),
    }
}

pub fn save_registry_to_path(path: &Path, registry: &RegistryFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create registry directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(registry).map_err(|e| format!("Serialize registry: {e}"))?;
    fs::write(path, json + "\n").map_err(|e| format!("Write registry: {e}"))
}

pub fn load_registry_from_path(path: &Path) -> Result<RegistryFile, String> {
    if !path.exists() {
        return Ok(empty_registry());
    }

    let raw = fs::read_to_string(path).map_err(|e| format!("Read registry: {e}"))?;
    let parsed: RegistryFile = serde_json::from_str(&raw).map_err(|e| format!("Parse registry: {e}"))?;
    Ok(parsed)
}

fn load_registry() -> Result<RegistryFile, String> {
    load_registry_from_path(&registry_path()?)
}

fn save_registry(registry: &RegistryFile) -> Result<(), String> {
    save_registry_to_path(&registry_path()?, registry)
}

fn find_claude_app() -> Result<Option<PathBuf>, String> {
    let candidates = [
        PathBuf::from("/Applications/Claude.app"),
        home_dir()?.join("Applications").join("Claude.app"),
    ];

    Ok(candidates.into_iter().find(|path| path.exists()))
}

pub fn list_extensions_in_dir(data_dir: &Path) -> Result<Vec<ExtensionEntry>, String> {
    let ext_dir = data_dir.join(EXT_DIR_NAME);
    let settings_dir = data_dir.join(EXT_SETTINGS_DIR_NAME);

    if !ext_dir.exists() {
        return Ok(Vec::new());
    }

    let mut extensions = Vec::new();
    for entry in fs::read_dir(&ext_dir).map_err(|e| format!("Read extension directory: {e}"))? {
        let entry = entry.map_err(|e| format!("Read extension entry: {e}"))?;
        if entry.file_type().map_err(|e| format!("Read extension file type: {e}"))?.is_dir() {
            let id = entry.file_name().to_string_lossy().to_string();
            extensions.push(ExtensionEntry {
                has_settings: settings_dir.join(format!("{id}.json")).exists(),
                id,
            });
        }
    }

    extensions.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(extensions)
}

fn safe_extension_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains('/')
        && !id.contains('\\')
        && id != "."
        && id != ".."
        && !id.split('.').any(|part| part == "..")
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| format!("Create target directory: {e}"))?;

    for entry in fs::read_dir(source).map_err(|e| format!("Read source directory: {e}"))? {
        let entry = entry.map_err(|e| format!("Read source entry: {e}"))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type().map_err(|e| format!("Read source file type: {e}"))?;

        if file_type.is_symlink() {
            // Preserve symlinks (e.g. a skill linked in from a plugin cache)
            // by recreating the link. fs::copy can't handle a symlink-to-dir
            // and errors with "neither a regular file nor a symlink to a
            // regular file" — that was breaking seeding entirely.
            if let Ok(link_target) = fs::read_link(&source_path) {
                let _ = std::os::unix::fs::symlink(&link_target, &target_path);
            }
        } else if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)
                .map_err(|e| format!("Copy {}: {e}", source_path.display()))?;
        }
    }

    Ok(())
}

fn remove_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || meta.is_file() => {
            fs::remove_file(path).map_err(|e| format!("Remove {}: {e}", path.display()))
        }
        Ok(meta) if meta.is_dir() => {
            fs::remove_dir_all(path).map_err(|e| format!("Remove {}: {e}", path.display()))
        }
        Ok(_) => fs::remove_file(path).map_err(|e| format!("Remove {}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Inspect {}: {e}", path.display())),
    }
}

/// Guarded recursive delete for a profile's DATA directory. All profile-data
/// erases route through here so a corrupted registry / bad caller can never
/// `rm -rf` the home directory, root, or a too-shallow path. Refuses anything
/// that is $HOME, "/", an ancestor of $HOME, or fewer than 2 levels under $HOME.
fn remove_data_dir(path: &Path) -> Result<(), String> {
    let home = home_dir()?;
    let canon = match path.canonicalize() {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()), // already gone
        Err(e) => return Err(format!("Canonicalize {}: {e}", path.display())),
    };
    if canon == home || canon == Path::new("/") {
        return Err("Refusing to delete the home or root directory.".to_string());
    }
    if home.starts_with(&canon) {
        return Err("Refusing to delete an ancestor of the home directory.".to_string());
    }
    // Must live under $HOME. Require >=2 components deep (e.g. ~/Library/.../X)
    // so a bad 1-deep path like ~/Documents or ~/.ssh can never be erased — EXCEPT
    // our own managed agent homes ~/.codex-<name> / ~/.claude-<name>, which are
    // legitimately 1 component deep (allow only those by exact dotdir prefix).
    match canon.strip_prefix(&home) {
        Ok(rel) => {
            let comps = rel.components().count();
            let first = rel
                .components()
                .next()
                .and_then(|c| c.as_os_str().to_str())
                .unwrap_or("");
            let allowed_shallow =
                first.starts_with(".codex-") || first.starts_with(".claude-");
            if comps >= 2 || (comps == 1 && allowed_shallow) {
                remove_path(&canon)
            } else {
                Err(format!(
                    "Refusing to delete {} — too shallow under the home directory.",
                    canon.display()
                ))
            }
        }
        _ => Err(format!(
            "Refusing to delete {} — not safely under the home directory.",
            canon.display()
        )),
    }
}

/// Validate a content file/dir path for read/write/delete: its parent must
/// resolve to a real directory strictly UNDER $HOME (never $HOME itself or
/// outside). Bounds all content editing to profile dirs; paths come from trusted
/// matrix cell.data_dir values, this is defense-in-depth.
fn guarded_file_path(raw: &str) -> Result<PathBuf, String> {
    let home0 = home_dir()?;
    let home = fs::canonicalize(&home0).unwrap_or(home0);
    let p = PathBuf::from(raw);
    let parent = p.parent().ok_or_else(|| "Path has no parent directory.".to_string())?;
    let fname = p.file_name().ok_or_else(|| "Path has no file name.".to_string())?;
    let cparent = parent
        .canonicalize()
        .map_err(|e| format!("Resolve {}: {e}", parent.display()))?;
    let full = cparent.join(fname);

    // Which managed root (if any) is this under? First component below $HOME
    // begins with ".claude" / ".codex".
    let rel = cparent.strip_prefix(&home);
    let first = rel
        .as_ref()
        .ok()
        .and_then(|r| r.components().next())
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or("");
    let in_managed_root = first.starts_with(".claude") || first.starts_with(".codex");

    // Inside our own profile dirs: broad allowance — symlinks are fine here
    // because that's exactly how content sharing works (a shared file IS a link).
    if in_managed_root {
        return Ok(full);
    }

    // Outside the managed roots: the ONLY thing we'll touch is a memory file
    // (CLAUDE.md / AGENTS.md) at a project root — they commonly live in repos, not
    // in ~/.claude. Tightly gated: must be under $HOME, must NOT sit in a hidden
    // dot-directory (keeps ~/.ssh/CLAUDE.md etc. out), and the file itself must
    // not be a symlink (no CLAUDE.md → ~/.ssh/id_rsa read/write/delete trick).
    let fname_str = fname.to_str().unwrap_or("");
    let is_memory_file = matches!(
        fname_str,
        "CLAUDE.md" | "CLAUDE.local.md" | "AGENTS.md" | "AGENTS.local.md"
    );
    if is_memory_file {
        let rel = rel.map_err(|_| {
            format!("Refusing to touch {} — outside your home directory.", p.display())
        })?;
        if rel
            .components()
            .any(|c| c.as_os_str().to_str().map_or(true, |s| s.starts_with('.')))
        {
            return Err(format!(
                "Refusing to touch {} — inside a hidden directory.",
                p.display()
            ));
        }
        if fs::symlink_metadata(&full)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(format!("Refusing to follow symlink {}.", full.display()));
        }
        return Ok(full);
    }

    Err(format!(
        "Refusing to touch {} — not inside a Claude/Codex profile directory.",
        p.display()
    ))
}

/// Read a content file (memory / SKILL.md / etc). Missing → "" (so the editor
/// can seed an empty buffer for "create").
pub fn read_text_file(path: String) -> Result<String, String> {
    let p = guarded_file_path(&path)?;
    match fs::read_to_string(&p) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("Read {}: {e}", p.display())),
    }
}

/// Write a content file. Refuses to write THROUGH a symlink (a shared file) — the
/// user must un-share first, so an edit never silently propagates to every linked
/// account. Creates the parent + file when absent (memory "create-empty").
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    let p = guarded_file_path(&path)?;
    if fs::symlink_metadata(&p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(
            "This file is shared (a symlink) — un-share it first to edit it independently."
                .to_string(),
        );
    }
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create {}: {e}", parent.display()))?;
    }
    fs::write(&p, content).map_err(|e| format!("Write {}: {e}", p.display()))
}

/// Copy memory CONTENT from one file into another — account→account
/// (CLAUDE.md→CLAUDE.md) or cross-tool (CLAUDE.md↔AGENTS.md), either direction.
/// A plain copy: the target becomes an independent file (not a symlink), so the
/// two memories can diverge afterward. Both ends are guarded memory paths.
pub fn import_memory_file(source: String, target: String) -> Result<(), String> {
    let src = guarded_file_path(&source)?;
    let content = match fs::read_to_string(&src) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(format!("Nothing to import — {} doesn't exist.", src.display()))
        }
        Err(e) => return Err(format!("Read {}: {e}", src.display())),
    };
    let dst = guarded_file_path(&target)?;
    if fs::symlink_metadata(&dst)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(
            "Target is shared (a symlink) — un-share it first so the import stays independent."
                .to_string(),
        );
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create {}: {e}", parent.display()))?;
    }
    fs::write(&dst, content).map_err(|e| format!("Write {}: {e}", dst.display()))
}

/// Delete a content file or folder (a skill dir, a session file, a memory file).
/// remove_path unlinks a symlink rather than touching its target.
pub fn delete_content_path(path: String) -> Result<(), String> {
    let p = guarded_file_path(&path)?;
    remove_path(&p)
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(h) = home_dir() {
            return h.join(rest);
        }
    }
    PathBuf::from(p)
}

/// Read one MCP server's config (pretty JSON) from a config file (JSON
/// mcpServers.<name> or TOML [mcp_servers.<name>]), detected by extension.
pub fn read_mcp_server(config_path: String, server: String) -> Result<String, String> {
    let path = expand_tilde(&config_path);
    let is_toml = path.extension().and_then(|e| e.to_str()) == Some("toml");
    let v = if is_toml {
        read_codex_mcp_at(&path).get(&server).cloned()
    } else {
        let raw = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|root| root.get("mcpServers").and_then(|m| m.get(&server)).cloned())
    };
    let v = v.ok_or_else(|| format!("MCP server \"{server}\" not found."))?;
    serde_json::to_string_pretty(&v).map_err(|e| format!("Serialize: {e}"))
}

/// Write one MCP server (keyed splice — never rewrites the rest of the file).
pub fn write_mcp_server(config_path: String, server: String, body: String) -> Result<(), String> {
    let path = expand_tilde(&config_path);
    let home = home_dir()?;
    if !path.starts_with(&home) {
        return Err("Refusing to write outside the home directory.".to_string());
    }
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Invalid JSON: {e}"))?;
    let is_toml = path.extension().and_then(|e| e.to_str()) == Some("toml");
    if is_toml {
        write_codex_mcp_server_at(&path, &server, &value)
    } else {
        let raw = fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
        let mut root: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("Parse {}: {e}", path.display()))?;
        let obj = root
            .as_object_mut()
            .ok_or_else(|| "Config is not a JSON object.".to_string())?;
        let servers = obj
            .entry("mcpServers")
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        servers
            .as_object_mut()
            .ok_or_else(|| "mcpServers is not an object.".to_string())?
            .insert(server, value);
        write_json_atomically(&path, &root)
    }
}

/// Delete one MCP server (keyed).
pub fn delete_mcp_server(config_path: String, server: String) -> Result<(), String> {
    let path = expand_tilde(&config_path);
    let is_toml = path.extension().and_then(|e| e.to_str()) == Some("toml");
    if is_toml {
        remove_codex_mcp_server_at(&path, &server).map(|_| ())
    } else {
        let raw = match fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => return Ok(()),
        };
        let mut root: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("Parse {}: {e}", path.display()))?;
        if let Some(servers) = root
            .as_object_mut()
            .and_then(|o| o.get_mut("mcpServers"))
            .and_then(|m| m.as_object_mut())
        {
            servers.remove(&server);
        }
        write_json_atomically(&path, &root)
    }
}

/// The home dir backing a session column id, for the given world.
fn session_home_for(install_id: &str, world: &str) -> Result<PathBuf, String> {
    if world == "codex" {
        codex_home_for_column(install_id)?.ok_or_else(|| format!("Unknown Codex account: {install_id}"))
    } else {
        claude_config_for_column(install_id)?.ok_or_else(|| format!("Unknown Claude account: {install_id}"))
    }
}

/// Read-only markdown transcript of one session (for the content viewer).
pub fn get_session_transcript(
    install_id: String,
    session_id: String,
    world: String,
) -> Result<String, String> {
    let home = session_home_for(&install_id, &world)?;
    convert::session_transcript(&session_id, &home, &world)
}

/// Delete a single session file (guarded). Never the project dir.
pub fn delete_session_file(
    install_id: String,
    session_id: String,
    world: String,
) -> Result<(), String> {
    let home = session_home_for(&install_id, &world)?;
    let file = convert::resolve_session_file(&session_id, &home, &world)
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    // Hard guard: only ever delete a single regular session FILE, never a dir.
    // (resolve's exists()-passthrough could otherwise return a project/sessions
    // tree → recursive wipe.) Require a real file ending in .jsonl.
    let p = Path::new(&file);
    let is_file = fs::symlink_metadata(p)
        .map(|m| m.is_file() || m.file_type().is_symlink())
        .unwrap_or(false);
    if !is_file || p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return Err("Refusing to delete — not a single session file.".to_string());
    }
    delete_content_path(file)
}

fn path_points_to(path: &Path, target: &Path) -> bool {
    let Ok(link) = fs::read_link(path) else {
        return false;
    };
    if link == target {
        return true;
    }
    let base = path.parent().unwrap_or_else(|| Path::new("/"));
    base.join(link) == target
}

fn backup_existing_path(path: &Path, data_dir: &Path, extension_id: &str) -> Result<(), String> {
    if !path.exists() && fs::symlink_metadata(path).is_err() {
        return Ok(());
    }

    let stamp = Utc::now().format("%Y%m%d-%H%M%S%3f");
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(extension_id);
    let backup_dir = data_dir
        .join("Claude Multiprofile Backups")
        .join(format!("{extension_id}-{stamp}"));
    fs::create_dir_all(&backup_dir).map_err(|e| format!("Create backup directory: {e}"))?;
    fs::rename(path, backup_dir.join(file_name)).map_err(|e| format!("Back up {}: {e}", path.display()))
}

#[cfg(unix)]
fn symlink_path(source: &Path, target: &Path) -> Result<(), String> {
    unix_fs::symlink(source, target)
        .map_err(|e| format!("Link {} -> {}: {e}", target.display(), source.display()))
}

#[cfg(not(unix))]
fn symlink_path(_source: &Path, _target: &Path) -> Result<(), String> {
    Err("Sharing requires symlink support on macOS.".to_string())
}

fn extension_paths(data_dir: &Path, extension_id: &str) -> (PathBuf, PathBuf) {
    (
        data_dir.join(EXT_DIR_NAME).join(extension_id),
        data_dir
            .join(EXT_SETTINGS_DIR_NAME)
            .join(format!("{extension_id}.json")),
    )
}

fn share_extension_one_way(source_data_dir: &Path, target_data_dir: &Path, extension_id: &str) -> Result<(), String> {
    let (source_folder, source_settings) = extension_paths(source_data_dir, extension_id);
    let (target_folder, target_settings) = extension_paths(target_data_dir, extension_id);
    if !source_folder.exists() {
        return Err(format!("Extension not found in source: {extension_id}"));
    }

    fs::create_dir_all(target_folder.parent().unwrap())
        .map_err(|e| format!("Create target extension directory: {e}"))?;
    fs::create_dir_all(target_settings.parent().unwrap())
        .map_err(|e| format!("Create target settings directory: {e}"))?;

    if !path_points_to(&target_folder, &source_folder) {
        backup_existing_path(&target_folder, target_data_dir, extension_id)?;
        symlink_path(&source_folder, &target_folder)?;
    }

    if source_settings.exists() {
        if !path_points_to(&target_settings, &source_settings) {
            backup_existing_path(&target_settings, target_data_dir, extension_id)?;
            symlink_path(&source_settings, &target_settings)?;
        }
    } else if target_settings.exists() || fs::symlink_metadata(&target_settings).is_ok() {
        backup_existing_path(&target_settings, target_data_dir, extension_id)?;
    }

    Ok(())
}

fn make_extension_independent_one_way(source_data_dir: &Path, target_data_dir: &Path, extension_id: &str) -> Result<bool, String> {
    let (source_folder, source_settings) = extension_paths(source_data_dir, extension_id);
    let (target_folder, target_settings) = extension_paths(target_data_dir, extension_id);
    let mut changed = false;

    if path_points_to(&target_folder, &source_folder) {
        remove_path(&target_folder)?;
        copy_dir_recursive(&source_folder, &target_folder)?;
        changed = true;
    }

    if path_points_to(&target_settings, &source_settings) {
        remove_path(&target_settings)?;
        if source_settings.exists() {
            fs::copy(&source_settings, &target_settings)
                .map_err(|e| format!("Copy independent settings: {e}"))?;
        }
        changed = true;
    }

    Ok(changed)
}

pub fn copy_extension_between_dirs(
    source_data_dir: &Path,
    target_data_dir: &Path,
    extension_id: &str,
) -> Result<(), String> {
    if !safe_extension_id(extension_id) {
        return Err(format!("Unsafe extension id: {extension_id}"));
    }

    let source_folder = source_data_dir.join(EXT_DIR_NAME).join(extension_id);
    if !source_folder.is_dir() {
        return Err(format!("Extension not found in source: {extension_id}"));
    }

    let target_ext_dir = target_data_dir.join(EXT_DIR_NAME);
    let target_settings_dir = target_data_dir.join(EXT_SETTINGS_DIR_NAME);
    fs::create_dir_all(&target_ext_dir).map_err(|e| format!("Create target extensions directory: {e}"))?;
    fs::create_dir_all(&target_settings_dir)
        .map_err(|e| format!("Create target extension settings directory: {e}"))?;

    let target_folder = target_ext_dir.join(extension_id);
    if target_folder.exists() {
        fs::remove_dir_all(&target_folder).map_err(|e| format!("Remove old target extension: {e}"))?;
    }
    copy_dir_recursive(&source_folder, &target_folder)?;

    let source_settings = source_data_dir
        .join(EXT_SETTINGS_DIR_NAME)
        .join(format!("{extension_id}.json"));
    if source_settings.exists() {
        let target_settings = target_settings_dir.join(format!("{extension_id}.json"));
        fs::copy(&source_settings, &target_settings)
            .map_err(|e| format!("Copy extension settings: {e}"))?;
    }

    Ok(())
}

pub fn build_extension_library(installs: &[DesktopInstall]) -> Result<Vec<ExtensionShareItem>, String> {
    let mut by_id: BTreeMap<String, Vec<(DesktopInstall, ExtensionEntry)>> = BTreeMap::new();
    let mut inventory_by_install = Vec::new();

    for install in installs {
        let extensions = list_extensions_in_dir(Path::new(&install.data_dir))?;
        for extension in &extensions {
            by_id
                .entry(extension.id.clone())
                .or_default()
                .push((install.clone(), extension.clone()));
        }
        inventory_by_install.push((install, extensions));
    }

    let mut items = Vec::new();
    for (id, sources) in by_id {
        let source_rows = sources
            .iter()
            .map(|(install, extension)| ExtensionLibrarySource {
                install_id: install.id.clone(),
                install_name: install.name.clone(),
                data_dir: install.data_dir.clone(),
                has_settings: extension.has_settings,
            })
            .collect();

        let targets = inventory_by_install
            .iter()
            .map(|(install, extensions)| {
                let existing = extensions.iter().find(|extension| extension.id == id);
                ExtensionTargetStatus {
                    install_id: install.id.clone(),
                    install_name: install.name.clone(),
                    data_dir: install.data_dir.clone(),
                    kind: install.kind.clone(),
                    has_extension: existing.is_some(),
                    has_settings: existing.is_some_and(|extension| extension.has_settings),
                }
            })
            .collect();

        items.push(ExtensionShareItem {
            id,
            sources: source_rows,
            targets,
        });
    }

    Ok(items)
}

pub fn copy_extension_to_target_dirs(
    source_data_dir: &Path,
    target_data_dirs: &[PathBuf],
    extension_id: &str,
) -> Result<CopySummary, String> {
    let mut copied = 0;
    let mut skipped = 0;

    for target in target_data_dirs {
        if target == source_data_dir {
            skipped += 1;
            continue;
        }
        copy_extension_between_dirs(source_data_dir, target, extension_id)?;
        copied += 1;
    }

    Ok(CopySummary { copied, skipped })
}

pub fn list_pair_extension_shares(
    source_data_dir: &Path,
    target_data_dir: &Path,
) -> Result<Vec<PairExtensionShare>, String> {
    let source_extensions = list_extensions_in_dir(source_data_dir)?;
    let target_extensions = list_extensions_in_dir(target_data_dir)?;
    let mut ids = BTreeMap::new();
    for extension in &source_extensions {
        ids.insert(extension.id.clone(), ());
    }
    for extension in &target_extensions {
        ids.insert(extension.id.clone(), ());
    }

    let mut rows = Vec::new();
    for id in ids.keys() {
        let source = source_extensions.iter().find(|extension| extension.id == *id);
        let target = target_extensions.iter().find(|extension| extension.id == *id);
        let (source_folder, source_settings) = extension_paths(source_data_dir, id);
        let (target_folder, target_settings) = extension_paths(target_data_dir, id);
        let target_to_source = path_points_to(&target_folder, &source_folder);
        let source_to_target = path_points_to(&source_folder, &target_folder);
        let settings_target_to_source = path_points_to(&target_settings, &source_settings);
        let settings_source_to_target = path_points_to(&source_settings, &target_settings);
        let folder_shared = target_to_source || source_to_target;
        let settings_relevant = source_settings.exists() || target_settings.exists();
        let settings_shared = !settings_relevant || settings_target_to_source || settings_source_to_target;

        rows.push(PairExtensionShare {
            id: id.clone(),
            source_has_extension: source.is_some(),
            target_has_extension: target.is_some(),
            source_has_settings: source.is_some_and(|extension| extension.has_settings),
            target_has_settings: target.is_some_and(|extension| extension.has_settings),
            shared: folder_shared && settings_shared,
            partial: folder_shared && !settings_shared,
            direction: if target_to_source {
                "source-to-target".to_string()
            } else if source_to_target {
                "target-to-source".to_string()
            } else {
                "independent".to_string()
            },
        });
    }

    Ok(rows)
}

pub fn set_pair_extension_shared(
    source_data_dir: &Path,
    target_data_dir: &Path,
    extension_id: &str,
    shared: bool,
) -> Result<bool, String> {
    if !safe_extension_id(extension_id) {
        return Err(format!("Unsafe extension id: {extension_id}"));
    }

    if shared {
        let (source_folder, _) = extension_paths(source_data_dir, extension_id);
        let (target_folder, _) = extension_paths(target_data_dir, extension_id);
        if source_folder.exists() {
            share_extension_one_way(source_data_dir, target_data_dir, extension_id)?;
            return Ok(true);
        }
        if target_folder.exists() {
            share_extension_one_way(target_data_dir, source_data_dir, extension_id)?;
            return Ok(true);
        }
        return Err(format!("Extension not found in either profile: {extension_id}"));
    }

    let changed_a = make_extension_independent_one_way(source_data_dir, target_data_dir, extension_id)?;
    let changed_b = make_extension_independent_one_way(target_data_dir, source_data_dir, extension_id)?;
    Ok(changed_a || changed_b)
}

fn install_from_default() -> Result<Option<DesktopInstall>, String> {
    let Some(app_path) = find_claude_app()? else {
        return Ok(None);
    };
    let data_dir = default_desktop_data_dir()?;
    if !data_dir.exists() {
        return Ok(None);
    }

    Ok(Some(DesktopInstall {
        id: "default".to_string(),
        name: "default".to_string(),
        kind: "default".to_string(),
        data_dir: data_dir.to_string_lossy().to_string(),
        app_path: Some(app_path.to_string_lossy().to_string()),
        launcher_path: None,
        managed: false,
        is_running: false,
    }))
}

fn install_from_profile(profile: &RegistryProfile) -> Option<DesktopInstall> {
    profile.desktop.as_ref().map(|desktop| DesktopInstall {
        id: format!("profile:{}", profile.name),
        name: profile.name.clone(),
        kind: "profile".to_string(),
        data_dir: desktop.data_dir.clone(),
        app_path: Some(desktop.claude_app_path.clone()),
        launcher_path: Some(desktop.app_path.clone()),
        managed: true,
        is_running: false,
    })
}

/// Read currently-running Claude.app instances by parsing `ps -A -o command`.
/// Each Claude.app launched with `--user-data-dir=<path>` corresponds to a
/// managed profile; a Claude.app launched without that flag is the default
/// install. Returns the set of data_dir strings that are live right now.
///
/// macOS `ps` truncates long lines unless you ask for `-ww` and a wide
/// format, so we use `args` (full command line) with no width limit.
fn detect_running_data_dirs() -> Vec<PathBuf> {
    let out = match Command::new("/bin/ps")
        .args(["-Aww", "-o", "args="])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let raw = String::from_utf8_lossy(&out.stdout);
    let default_dir = default_desktop_data_dir().ok();
    let mut running: Vec<PathBuf> = Vec::new();

    for line in raw.lines() {
        // Only the top-level Claude.app binary, not helper / renderer /
        // GPU subprocesses (they don't carry the user-data-dir arg anyway,
        // but skipping them keeps us robust to future arg additions).
        let trimmed = line.trim_start();
        if !trimmed.contains("/Claude.app/Contents/MacOS/Claude") {
            continue;
        }
        if trimmed.contains("Helper")
            || trimmed.contains("Renderer")
            || trimmed.contains("Crashpad")
            || trimmed.contains("GPU")
            || trimmed.contains("Utility")
        {
            continue;
        }
        // Pull `--user-data-dir=` argument. The path can contain spaces
        // (e.g. "Application Support"), so we cannot just split on space —
        // the arg occupies the rest of the line up to the next flag (which
        // would start with " --"). In practice Desktop emits it last.
        if let Some(idx) = trimmed.find("--user-data-dir=") {
            let after = &trimmed[idx + "--user-data-dir=".len()..];
            // If a subsequent flag exists, cut before it.
            let path_str = after
                .find(" --")
                .map(|j| &after[..j])
                .unwrap_or(after)
                .trim()
                .trim_end_matches('\0');
            running.push(PathBuf::from(path_str));
        } else if let Some(d) = &default_dir {
            // Claude.app launched with no flag → default install.
            running.push(d.clone());
        }
    }
    running
}

pub fn list_desktop_installs() -> Result<Vec<DesktopInstall>, String> {
    let mut installs = Vec::new();
    if let Some(default) = install_from_default()? {
        installs.push(default);
    }

    let registry = load_registry()?;
    for profile in &registry.profiles {
        if let Some(install) = install_from_profile(profile) {
            installs.push(install);
        }
    }

    // Tag each install with is_running by canonical-path matching against
    // the currently-running Claude.app instances.
    let running_paths = detect_running_data_dirs();
    let running_canon: Vec<PathBuf> = running_paths
        .iter()
        .filter_map(|p| fs::canonicalize(p).ok().or_else(|| Some(p.clone())))
        .collect();
    for install in &mut installs {
        let mine_raw = PathBuf::from(&install.data_dir);
        let mine_canon = fs::canonicalize(&mine_raw).unwrap_or(mine_raw);
        install.is_running = running_canon.iter().any(|p| p == &mine_canon);
    }

    Ok(installs)
}

fn run_command(mut command: Command, context: &str) -> Result<(), String> {
    let output = command.output().map_err(|e| format!("{context}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "{context} failed: {}{}",
        stderr.trim(),
        if stdout.trim().is_empty() {
            String::new()
        } else {
            format!(" {}", stdout.trim())
        }
    ))
}

fn shell_quote_single(value: &Path) -> String {
    value.to_string_lossy().replace('\'', "'\\''")
}

/// Build the launcher's AppleScript. For Claude (Electron) `--user-data-dir`
/// isolates the whole account. For Codex (CEF) `--user-data-dir` isolates only
/// the Chromium/web layer; the OAuth token lives in `$CODEX_HOME/auth.json`
/// (default `~/.codex`, shared by every instance), so a managed Codex profile
/// must ALSO pin its own `CODEX_HOME` via `open --env` or it shows the same
/// account. `--env` must precede `--args` (which swallows the rest).
fn build_launch_applescript(
    data_dir: &Path,
    app_path: &Path,
    codex_home: Option<&Path>,
) -> String {
    let safe_app = shell_quote_single(app_path);
    let safe_dir = shell_quote_single(data_dir);
    let env_flag = match codex_home {
        Some(home) => format!(" --env CODEX_HOME='{}'", shell_quote_single(home)),
        None => String::new(),
    };
    format!(
        "do shell script \"open -n -a '{}'{} --args --user-data-dir='{}' > /dev/null 2>&1 &\"",
        safe_app, env_flag, safe_dir
    )
}

fn unique_bundle_id(name: &str) -> String {
    let safe = sanitize_profile_name(name);
    format!(
        "com.claude-multiprofile.{}",
        if safe.is_empty() { "profile" } else { safe.as_str() }
    )
}

fn set_bundle_id(app_path: &Path, bundle_id: &str) {
    let plist = app_path.join("Contents").join("Info.plist");
    if !plist.exists() {
        return;
    }

    let mut set = Command::new("/usr/libexec/PlistBuddy");
    set.args(["-c", &format!("Set :CFBundleIdentifier {bundle_id}")])
        .arg(&plist);
    if run_command(set, "Set bundle identifier").is_ok() {
        return;
    }

    let mut add = Command::new("/usr/libexec/PlistBuddy");
    add.args(["-c", &format!("Add :CFBundleIdentifier string {bundle_id}")])
        .arg(plist);
    let _ = run_command(add, "Add bundle identifier");
}

fn compile_launcher_app(
    name: &str,
    data_dir: &Path,
    app_path: &Path,
    target_app_path: &Path,
    codex_home: Option<&Path>,
) -> Result<(), String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Read system time: {e}"))?
        .as_nanos();
    let tmp_dir = env::temp_dir().join(format!("claude-multiprofile-{nanos}"));
    fs::create_dir_all(&tmp_dir).map_err(|e| format!("Create temp directory: {e}"))?;
    let script_path = tmp_dir.join("launcher.applescript");
    fs::write(
        &script_path,
        build_launch_applescript(data_dir, target_app_path, codex_home),
    )
    .map_err(|e| format!("Write launcher script: {e}"))?;

    if let Some(parent) = app_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create app parent directory: {e}"))?;
    }
    if app_path.exists() {
        fs::remove_dir_all(app_path).map_err(|e| format!("Remove existing launcher: {e}"))?;
    }

    let mut osacompile = Command::new("/usr/bin/osacompile");
    osacompile.args(["-o"]).arg(app_path).arg(&script_path);
    let result = run_command(osacompile, "Compile launcher app");
    let _ = fs::remove_dir_all(&tmp_dir);
    result?;

    set_bundle_id(app_path, &unique_bundle_id(name));
    strip_quarantine(app_path);
    copy_claude_icon(app_path, target_app_path);
    Ok(())
}

fn strip_quarantine(app_path: &Path) {
    let mut xattr = Command::new("/usr/bin/xattr");
    xattr.args(["-dr", "com.apple.quarantine"]).arg(app_path);
    let _ = run_command(xattr, "Strip quarantine");
}

fn copy_claude_icon(app_path: &Path, claude_app_path: &Path) {
    let source_resources = claude_app_path.join("Contents").join("Resources");
    let target_icon = app_path
        .join("Contents")
        .join("Resources")
        .join("applet.icns");
    if !source_resources.is_dir() || !target_icon.exists() {
        return;
    }

    let Ok(entries) = fs::read_dir(source_resources) else {
        return;
    };
    for entry in entries.flatten() {
        let source = entry.path();
        if source
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("icns"))
        {
            let _ = fs::copy(source, &target_icon);
            let mut touch = Command::new("/usr/bin/touch");
            touch.arg(app_path);
            let _ = run_command(touch, "Refresh launcher icon");
            break;
        }
    }
}

pub fn create_desktop_profile(name: String) -> Result<DesktopInstall, String> {
    let clean_name = sanitize_profile_name(&name);
    if clean_name.is_empty() {
        return Err("Profile name cannot be empty".to_string());
    }

    let claude_app_path = find_claude_app()?.ok_or_else(|| {
        "Claude.app was not found in /Applications or ~/Applications".to_string()
    })?;
    let data_dir = default_data_dir_for(&clean_name)?;
    if data_dir == default_desktop_data_dir()? {
        return Err("Refusing to use the default Claude data directory".to_string());
    }

    let app_path = default_app_path_for(&clean_name)?;
    let mut registry = load_registry()?;
    if registry.profiles.iter().any(|profile| profile.name == clean_name) {
        return Err(format!("Profile \"{clean_name}\" already exists"));
    }

    fs::create_dir_all(&data_dir).map_err(|e| format!("Create profile data directory: {e}"))?;
    compile_launcher_app(&clean_name, &data_dir, &app_path, &claude_app_path, None)?;

    registry.profiles.push(RegistryProfile {
        name: clean_name.clone(),
        profile_type: "desktop".to_string(),
        desktop: Some(RegistryDesktop {
            data_dir: data_dir.to_string_lossy().to_string(),
            app_path: app_path.to_string_lossy().to_string(),
            claude_app_path: claude_app_path.to_string_lossy().to_string(),
        }),
        code: None,
        codex: None,
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    });
    save_registry(&registry)?;

    Ok(DesktopInstall {
        id: format!("profile:{clean_name}"),
        name: clean_name,
        kind: "profile".to_string(),
        data_dir: data_dir.to_string_lossy().to_string(),
        app_path: Some(claude_app_path.to_string_lossy().to_string()),
        launcher_path: Some(app_path.to_string_lossy().to_string()),
        managed: true,
        is_running: false,
    })
}

pub fn launch_desktop_install(install_id: String) -> Result<(), String> {
    let install = list_desktop_installs()?
        .into_iter()
        .find(|install| install.id == install_id)
        .ok_or_else(|| format!("Install not found: {install_id}"))?;

    if install.kind == "default" {
        let app = install.app_path.ok_or_else(|| "Default app path is missing".to_string())?;
        let mut open = Command::new("/usr/bin/open");
        open.arg(app);
        return run_command(open, "Launch Claude Desktop");
    }

    if let Some(launcher) = install.launcher_path.as_ref().filter(|path| Path::new(path).exists()) {
        let mut open = Command::new("/usr/bin/open");
        open.arg(launcher);
        return run_command(open, "Launch Claude profile");
    }

    let app = install.app_path.ok_or_else(|| "Claude.app source is missing".to_string())?;
    let mut open = Command::new("/usr/bin/open");
    // `=` form (single token) so detect_running_data_dirs' `--user-data-dir=`
    // needle matches the live process argv.
    open.args(["-n", "-a", &app, "--args", &format!("--user-data-dir={}", install.data_dir)]);
    run_command(open, "Launch Claude profile")
}

pub fn list_extension_matrix(
    source_data_dir: String,
    target_data_dir: String,
) -> Result<Vec<ExtensionSelectionRow>, String> {
    let source = PathBuf::from(source_data_dir);
    let target = PathBuf::from(target_data_dir);
    let source_extensions = list_extensions_in_dir(&source)?;
    let target_extensions = list_extensions_in_dir(&target)?;
    let target_ids: HashSet<_> = target_extensions.iter().map(|ext| ext.id.as_str()).collect();
    let target_settings_ids: HashSet<_> = target_extensions
        .iter()
        .filter(|ext| ext.has_settings)
        .map(|ext| ext.id.as_str())
        .collect();

    Ok(source_extensions
        .into_iter()
        .map(|ext| ExtensionSelectionRow {
            exists_in_target: target_ids.contains(ext.id.as_str()),
            target_has_settings: target_settings_ids.contains(ext.id.as_str()),
            has_settings: ext.has_settings,
            id: ext.id,
        })
        .collect())
}

pub fn copy_selected_extensions(
    source_data_dir: String,
    target_data_dir: String,
    extension_ids: Vec<String>,
) -> Result<CopySummary, String> {
    let source = PathBuf::from(source_data_dir);
    let target = PathBuf::from(target_data_dir);
    let mut copied = 0;
    let mut skipped = 0;

    for id in extension_ids {
        if list_extensions_in_dir(&source)?.iter().any(|ext| ext.id == id) {
            copy_extension_between_dirs(&source, &target, &id)?;
            copied += 1;
        } else {
            skipped += 1;
        }
    }

    Ok(CopySummary { copied, skipped })
}

pub fn list_extension_library() -> Result<Vec<ExtensionShareItem>, String> {
    build_extension_library(&list_desktop_installs()?)
}

pub fn copy_extension_to_targets(
    source_data_dir: String,
    target_data_dirs: Vec<String>,
    extension_id: String,
) -> Result<CopySummary, String> {
    let targets = target_data_dirs.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    copy_extension_to_target_dirs(Path::new(&source_data_dir), &targets, &extension_id)
}

pub fn list_pair_sharing(
    source_data_dir: String,
    target_data_dir: String,
) -> Result<Vec<PairExtensionShare>, String> {
    list_pair_extension_shares(Path::new(&source_data_dir), Path::new(&target_data_dir))
}

pub fn apply_pair_sharing(
    source_data_dir: String,
    target_data_dir: String,
    changes: Vec<PairShareChange>,
) -> Result<CopySummary, String> {
    let source = Path::new(&source_data_dir);
    let target = Path::new(&target_data_dir);
    let mut copied = 0;
    let mut skipped = 0;

    for change in changes {
        if set_pair_extension_shared(source, target, &change.extension_id, change.shared)? {
            copied += 1;
        } else {
            skipped += 1;
        }
    }

    Ok(CopySummary { copied, skipped })
}

// ---------------------------------------------------------------------------
// Desktop-embedded Claude Code history sharing
// ---------------------------------------------------------------------------
// Each Desktop install isolates the chat history of the embedded Claude Code
// panel under `<dataDir>/claude-code-sessions/<deviceId>/<workspaceId>/local_*.json`.
// Switching Desktop accounts therefore loses Code chat context. We expose a
// share at the per-workspace level (`<accountId>/<orgId>/`), NOT the whole
// `claude-code-sessions/` directory. Reverse-engineered from
// Claude.app's `LocalSessionManager.getStorageDir()`:
//
//     path.join(userDataPath, "claude-code-sessions",
//               currentAccountId, currentOrgId)
//
// Both IDs come from Anthropic's auth server and are stable per profile.
// We read them from plain-JSON files Claude Desktop writes on every launch
// (`cowork-enabled-cli-ops.json`, `extensions-blocklist.json`), so we can
// pre-create the target's `<acct>/<org>/` as a symlink at the source's
// even if the target hasn't actually used the Code panel yet — Desktop
// will then transparently read the source's sessions on first read.
//
// Login state (cookies, Local Storage with auth tokens) stays profile-local
// because we only touch the on-disk session folder, not the auth surface.

const DESKTOP_CODE_SESSIONS_DIR: &str = "claude-code-sessions";
const COWORK_OPS_FILE: &str = "cowork-enabled-cli-ops.json";
const EXTENSIONS_BLOCKLIST_FILE: &str = "extensions-blocklist.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DesktopCodeHistoryStat {
    pub present: bool,
    pub session_count: u32,
    pub total_bytes: u64,
    pub last_activity_ms: i64,
    /// Up to 5 distinct cwds, ordered by most-recent activity.
    pub recent_cwds: Vec<String>,
    /// `<accountId>/<orgId>` for the profile's current login. Read from
    /// plain-JSON files Desktop writes on every launch; no LevelDB needed.
    /// `None` means the profile hasn't logged in yet.
    pub primary_workspace: Option<DesktopCodeWorkspaceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopCodeWorkspaceRef {
    /// First subdir under `claude-code-sessions/`. Anthropic accountId.
    /// (Field name kept as `device_id` for backwards-compat with the
    /// 0.1.9 frontend; semantically it's the accountId.)
    pub device_id: String,
    /// Second subdir. Anthropic orgId.
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairDesktopCodeHistory {
    pub source: DesktopCodeHistoryStat,
    pub target: DesktopCodeHistoryStat,
    /// True iff target's primary workspace dir is a live symlink at source's.
    pub shared: bool,
    /// "source-to-target" (target's workspace is a link to source's),
    /// "target-to-source", or "independent".
    pub direction: String,
    /// True iff target has no `<dev>/<ws>/` workspace yet — sharing requires
    /// the user to launch Desktop on that profile and open the Code panel
    /// once so a workspace is generated.
    pub target_needs_bootstrap: bool,
    /// Same for source.
    pub source_needs_bootstrap: bool,
    /// True iff the legacy whole-`claude-code-sessions/` symlink is in place
    /// (older versions of this app). When set, applying any change will
    /// undo it before installing the workspace-level link.
    pub legacy_whole_dir_link: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PairDesktopCodeHistoryChange {
    pub shared: bool,
}

fn desktop_code_sessions_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DESKTOP_CODE_SESSIONS_DIR)
}

fn desktop_code_workspace_path(data_dir: &Path, ws: &DesktopCodeWorkspaceRef) -> PathBuf {
    desktop_code_sessions_path(data_dir)
        .join(&ws.device_id)
        .join(&ws.workspace_id)
}

/// Read the profile's Anthropic accountId from `cowork-enabled-cli-ops.json`.
/// Desktop rewrites this file on every launch, so it's our source of truth.
fn read_account_id(data_dir: &Path) -> Result<Option<String>, String> {
    let path = data_dir.join(COWORK_OPS_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Read {}: {e}", path.display())),
    };
    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    Ok(v.get("ownerAccountId")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

/// Read the profile's Anthropic orgId from `extensions-blocklist.json`.
/// The file always contains an entry whose URL embeds the org UUID, e.g.
/// `https://claude.ai/api/organizations/<orgId>/dxt/blocklist`.
fn read_org_id(data_dir: &Path) -> Result<Option<String>, String> {
    let path = data_dir.join(EXTENSIONS_BLOCKLIST_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Read {}: {e}", path.display())),
    };
    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let url = v
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("url"))
        .and_then(|u| u.as_str());
    let Some(url) = url else { return Ok(None) };
    // Pull the UUID right after `/organizations/`.
    let needle = "/organizations/";
    let Some(idx) = url.find(needle) else {
        return Ok(None);
    };
    let after = &url[idx + needle.len()..];
    let end = after.find('/').unwrap_or(after.len());
    let candidate = &after[..end];
    if candidate.len() == 36 && candidate.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        Ok(Some(candidate.to_string()))
    } else {
        Ok(None)
    }
}

/// True identity of the profile's Code workspace: read `<accountId>/<orgId>`
/// from JSON files Desktop maintains on every launch. This works even
/// before the profile has ever opened the Code panel — only login is
/// required.
fn read_workspace_identity(data_dir: &Path) -> Result<Option<DesktopCodeWorkspaceRef>, String> {
    let acct = read_account_id(data_dir)?;
    let org = read_org_id(data_dir)?;
    if let (Some(a), Some(o)) = (acct, org) {
        Ok(Some(DesktopCodeWorkspaceRef {
            device_id: a,
            workspace_id: o,
        }))
    } else {
        Ok(None)
    }
}

/// Walk every `<acct>/<org>/local_*.json` and collect aggregate stats.
/// Tolerant of half-written files: a single bad JSON is skipped, not fatal.
///
/// `data_dir` is the Desktop user-data dir, `root` is its
/// `claude-code-sessions/` subdirectory. They're separate args because
/// we read account/org identity from JSON files at the top of `data_dir`,
/// independent of whether `claude-code-sessions/` itself exists yet.
fn scan_desktop_code_history_with_data_dir(
    data_dir: &Path,
    root: &Path,
) -> Result<DesktopCodeHistoryStat, String> {
    let identity_from_files = read_workspace_identity(data_dir)?;
    let mut stat = scan_desktop_code_history_walk(root)?;
    // The on-disk dir scan picks the most-recently-active <acct>/<org> as
    // primary, but Desktop *only* ever writes to the currently-logged-in
    // identity. Use the JSON-file-derived identity when available — it
    // reflects the live login, which is what Desktop will read.
    if identity_from_files.is_some() {
        stat.primary_workspace = identity_from_files;
    }
    Ok(stat)
}

/// Convenience wrapper for tests + callers that only have the
/// `claude-code-sessions/` path. Falls back to the directory-walk-only
/// strategy.
#[cfg(test)]
fn scan_desktop_code_history(root: &Path) -> Result<DesktopCodeHistoryStat, String> {
    scan_desktop_code_history_walk(root)
}

fn scan_desktop_code_history_walk(root: &Path) -> Result<DesktopCodeHistoryStat, String> {
    let mut stat = DesktopCodeHistoryStat::default();
    let meta = match fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(stat),
        Err(e) => return Err(format!("Inspect {}: {e}", root.display())),
    };
    // Treat a symlink whose target is missing as not-present rather than erroring.
    if meta.file_type().is_symlink() && !root.exists() {
        return Ok(stat);
    }
    if !root.is_dir() {
        return Ok(stat);
    }
    stat.present = true;

    // (cwd, last_activity_ms) -> keep newest per cwd.
    let mut cwd_latest: BTreeMap<String, i64> = BTreeMap::new();
    // (dev, ws) -> (last_activity_ms, session_count). The primary workspace
    // is the one with the largest last_activity, ties broken by session count
    // and finally lexical order so the choice is deterministic.
    let mut workspace_stats: BTreeMap<(String, String), (i64, u32)> = BTreeMap::new();

    let device_iter = match fs::read_dir(root) {
        Ok(it) => it,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(stat),
        Err(e) => return Err(format!("Read {}: {e}", root.display())),
    };
    for dev_entry in device_iter {
        let dev_entry = match dev_entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let dev_path = dev_entry.path();
        if !dev_path.is_dir() {
            continue;
        }
        let dev_name = match dev_path.file_name().and_then(|s| s.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        let ws_iter = match fs::read_dir(&dev_path) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for ws_entry in ws_iter {
            let ws_entry = match ws_entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let ws_path = ws_entry.path();
            // Treat both real dirs and symlinks-to-dirs as workspaces.
            let target_meta = match fs::metadata(&ws_path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !target_meta.is_dir() {
                continue;
            }
            let ws_name = match ws_path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            // Make sure the workspace is recorded even if it has zero session
            // files yet — that empty shell is what we need for sharing.
            workspace_stats
                .entry((dev_name.clone(), ws_name.clone()))
                .or_insert((0, 0));
            let session_iter = match fs::read_dir(&ws_path) {
                Ok(it) => it,
                Err(_) => continue,
            };
            for session_entry in session_iter {
                let session_entry = match session_entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let path = session_entry.path();
                let meta = match session_entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if !meta.is_file() {
                    continue;
                }
                let is_json = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
                if !is_json {
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if !name.starts_with("local_") {
                    continue;
                }
                stat.session_count = stat.session_count.saturating_add(1);
                stat.total_bytes = stat.total_bytes.saturating_add(meta.len());

                let mut session_last_activity: i64 = 0;
                // Parse just enough to pull cwd + lastActivityAt.
                if let Ok(raw) = fs::read_to_string(&path) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                        let last_activity = v
                            .get("lastActivityAt")
                            .and_then(|x| x.as_i64())
                            .or_else(|| v.get("createdAt").and_then(|x| x.as_i64()))
                            .unwrap_or(0);
                        session_last_activity = last_activity;
                        if last_activity > stat.last_activity_ms {
                            stat.last_activity_ms = last_activity;
                        }
                        if let Some(cwd) = v.get("cwd").and_then(|x| x.as_str()) {
                            let trimmed = cwd.trim();
                            if !trimmed.is_empty() {
                                let entry = cwd_latest.entry(trimmed.to_string()).or_insert(0);
                                if last_activity > *entry {
                                    *entry = last_activity;
                                }
                            }
                        }
                    }
                }
                // Fall back to file mtime if the JSON had no timestamps.
                if session_last_activity == 0 {
                    if let Ok(modified) = meta.modified() {
                        let mtime = system_time_to_epoch_ms(modified);
                        session_last_activity = mtime;
                        if mtime > stat.last_activity_ms {
                            stat.last_activity_ms = mtime;
                        }
                    }
                }

                let entry = workspace_stats
                    .entry((dev_name.clone(), ws_name.clone()))
                    .or_insert((0, 0));
                if session_last_activity > entry.0 {
                    entry.0 = session_last_activity;
                }
                entry.1 = entry.1.saturating_add(1);
            }
        }
    }

    // Top 5 cwds by recency.
    let mut cwds: Vec<(String, i64)> = cwd_latest.into_iter().collect();
    cwds.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    stat.recent_cwds = cwds.into_iter().take(5).map(|(k, _)| k).collect();

    // Pick the primary workspace.
    if !workspace_stats.is_empty() {
        let mut entries: Vec<((String, String), (i64, u32))> = workspace_stats.into_iter().collect();
        entries.sort_by(|a, b| {
            // primary = highest last-activity, then highest session count,
            // then lexical (dev, ws) for determinism.
            b.1 .0
                .cmp(&a.1 .0)
                .then_with(|| b.1 .1.cmp(&a.1 .1))
                .then_with(|| a.0 .0.cmp(&b.0 .0))
                .then_with(|| a.0 .1.cmp(&b.0 .1))
        });
        let ((dev, ws), _) = entries.remove(0);
        stat.primary_workspace = Some(DesktopCodeWorkspaceRef {
            device_id: dev,
            workspace_id: ws,
        });
    }
    Ok(stat)
}

fn pair_desktop_code_history(
    source_data_dir: &Path,
    target_data_dir: &Path,
) -> Result<PairDesktopCodeHistory, String> {
    let source_sessions = desktop_code_sessions_path(source_data_dir);
    let target_sessions = desktop_code_sessions_path(target_data_dir);
    let source = scan_desktop_code_history_with_data_dir(source_data_dir, &source_sessions)?;
    let target = scan_desktop_code_history_with_data_dir(target_data_dir, &target_sessions)?;

    // Legacy: an older version of this app may have linked target's whole
    // `claude-code-sessions/` to source's. We surface that so the next apply
    // can clean it up.
    let legacy_whole_dir_link = path_points_to(&target_sessions, &source_sessions)
        || path_points_to(&source_sessions, &target_sessions);

    // Workspace-level link state: target's primary <dev>/<ws>/ → source's
    // primary <dev>/<ws>/.
    let mut target_to_source = false;
    let mut source_to_target = false;
    if let (Some(src_ws), Some(tgt_ws)) = (&source.primary_workspace, &target.primary_workspace) {
        let src_ws_path = desktop_code_workspace_path(source_data_dir, src_ws);
        let tgt_ws_path = desktop_code_workspace_path(target_data_dir, tgt_ws);
        target_to_source = path_points_to(&tgt_ws_path, &src_ws_path);
        source_to_target = path_points_to(&src_ws_path, &tgt_ws_path);
    }

    let direction = if target_to_source {
        "source-to-target"
    } else if source_to_target {
        "target-to-source"
    } else {
        "independent"
    }
    .to_string();

    Ok(PairDesktopCodeHistory {
        target_needs_bootstrap: target.primary_workspace.is_none(),
        source_needs_bootstrap: source.primary_workspace.is_none(),
        source,
        target,
        shared: target_to_source || source_to_target,
        direction,
        legacy_whole_dir_link,
    })
}

/// If a previous version of this app symlinked target's whole
/// `claude-code-sessions/` directory at source's, undo that link before we
/// install a workspace-level one. The link is replaced with an empty real
/// directory so Desktop is free to recreate `<dev>/<ws>/` inside it.
fn cleanup_legacy_whole_dir_link(
    source_data_dir: &Path,
    target_data_dir: &Path,
) -> Result<(), String> {
    let source_sessions = desktop_code_sessions_path(source_data_dir);
    let target_sessions = desktop_code_sessions_path(target_data_dir);

    // Case 1: target -> source.
    if path_points_to(&target_sessions, &source_sessions) {
        backup_existing_path(&target_sessions, target_data_dir, DESKTOP_CODE_SESSIONS_DIR)?;
        fs::create_dir_all(&target_sessions)
            .map_err(|e| format!("Recreate target claude-code-sessions: {e}"))?;
    }
    // Case 2: source -> target (rare; same treatment, but on the source side).
    if path_points_to(&source_sessions, &target_sessions) {
        backup_existing_path(&source_sessions, source_data_dir, DESKTOP_CODE_SESSIONS_DIR)?;
        fs::create_dir_all(&source_sessions)
            .map_err(|e| format!("Recreate source claude-code-sessions: {e}"))?;
    }
    Ok(())
}

fn share_desktop_code_history(
    source_data_dir: &Path,
    target_data_dir: &Path,
) -> Result<(), String> {
    cleanup_legacy_whole_dir_link(source_data_dir, target_data_dir)?;

    // Identities come from JSON files Desktop maintains on every launch.
    // No need to wait for the user to send a Code message — they only need
    // to have logged in once, which writes both files.
    let source_ws = read_workspace_identity(source_data_dir)?
        .ok_or_else(|| login_first_message("source", source_data_dir))?;
    let target_ws = read_workspace_identity(target_data_dir)?
        .ok_or_else(|| login_first_message("target", target_data_dir))?;

    let source_ws_path = desktop_code_workspace_path(source_data_dir, &source_ws);
    let target_ws_path = desktop_code_workspace_path(target_data_dir, &target_ws);

    // Ensure the source's <acct>/<org>/ exists. If it doesn't (the source
    // profile is logged in but has never used Code), create it empty so
    // the symlink has somewhere valid to point. Desktop will populate it
    // on first save from either side.
    fs::create_dir_all(&source_ws_path)
        .map_err(|e| format!("Create source workspace dir: {e}"))?;

    if path_points_to(&target_ws_path, &source_ws_path) {
        return Ok(());
    }

    // Pre-create target's `claude-code-sessions/<acct>/`, ready to receive
    // the symlink. Even if the target has never opened the Code panel,
    // this gives Desktop the path it expects on next read.
    if let Some(parent) = target_ws_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Create target workspace parent: {e}"))?;
    }
    // If a real `<acct>/<org>/` already exists on the target side (the
    // user did use Code in this profile), back it up — its session files
    // will be available under "Claude Multiprofile Backups" if they ever
    // want to recover them.
    backup_existing_path(
        &target_ws_path,
        target_data_dir,
        &format!(
            "{}-{}-{}",
            DESKTOP_CODE_SESSIONS_DIR, target_ws.device_id, target_ws.workspace_id
        ),
    )?;
    symlink_path(&source_ws_path, &target_ws_path)?;
    Ok(())
}

fn login_first_message(side: &str, data_dir: &Path) -> String {
    let acct = data_dir.join(COWORK_OPS_FILE);
    format!(
        "{} profile hasn't completed Claude Desktop login yet (missing {}). Launch Claude Desktop on this profile, finish login, then click Share again.",
        if side == "source" { "Source" } else { "Target" },
        acct.display()
    )
}

fn make_desktop_code_history_independent(
    source_data_dir: &Path,
    target_data_dir: &Path,
) -> Result<bool, String> {
    // Workspace-level unshare: only meaningful when target's primary workspace
    // is currently a symlink at source's primary workspace.
    let source_identity = read_workspace_identity(source_data_dir)?;
    let target_identity = read_workspace_identity(target_data_dir)?;
    let mut acted = false;
    if let (Some(src_ws), Some(tgt_ws)) = (
        source_identity.as_ref(),
        target_identity.as_ref(),
    ) {
        let src_ws_path = desktop_code_workspace_path(source_data_dir, src_ws);
        let tgt_ws_path = desktop_code_workspace_path(target_data_dir, tgt_ws);
        if path_points_to(&tgt_ws_path, &src_ws_path) {
            remove_path(&tgt_ws_path)?;
            if src_ws_path.is_dir() {
                copy_dir_recursive(&src_ws_path, &tgt_ws_path)?;
            } else if let Some(parent) = tgt_ws_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!("Recreate target workspace parent: {e}")
                })?;
                fs::create_dir_all(&tgt_ws_path)
                    .map_err(|e| format!("Recreate target workspace: {e}"))?;
            }
            acted = true;
        }
    }
    // Also clean up a legacy whole-dir link if one is still present.
    let source_sessions = desktop_code_sessions_path(source_data_dir);
    let target_sessions = desktop_code_sessions_path(target_data_dir);
    if path_points_to(&target_sessions, &source_sessions)
        || path_points_to(&source_sessions, &target_sessions)
    {
        cleanup_legacy_whole_dir_link(source_data_dir, target_data_dir)?;
        // After cleanup, copy source's content over so target ends up
        // independent rather than empty.
        let target_sessions_now = desktop_code_sessions_path(target_data_dir);
        if source_sessions.is_dir() && target_sessions_now.exists() {
            // copy_dir_recursive expects target absent; remove the empty dir
            // we just created in cleanup, then copy.
            if let Ok(meta) = fs::metadata(&target_sessions_now) {
                if meta.is_dir()
                    && fs::read_dir(&target_sessions_now)
                        .map(|mut it| it.next().is_none())
                        .unwrap_or(false)
                {
                    let _ = fs::remove_dir(&target_sessions_now);
                }
            }
            copy_dir_recursive(&source_sessions, &target_sessions_now)?;
        }
        acted = true;
    }
    Ok(acted)
}

pub fn list_pair_desktop_code_history(
    source_data_dir: String,
    target_data_dir: String,
) -> Result<PairDesktopCodeHistory, String> {
    pair_desktop_code_history(
        Path::new(&source_data_dir),
        Path::new(&target_data_dir),
    )
}

pub fn apply_pair_desktop_code_history(
    source_data_dir: String,
    target_data_dir: String,
    change: PairDesktopCodeHistoryChange,
) -> Result<CopySummary, String> {
    let source = Path::new(&source_data_dir);
    let target = Path::new(&target_data_dir);
    let mut copied = 0;
    let mut skipped = 0;
    let current = pair_desktop_code_history(source, target)?;
    if change.shared {
        if current.shared && !current.legacy_whole_dir_link {
            skipped += 1;
        } else {
            share_desktop_code_history(source, target)?;
            copied += 1;
        }
    } else {
        if !current.shared && !current.legacy_whole_dir_link {
            skipped += 1;
        } else if make_desktop_code_history_independent(source, target)? {
            copied += 1;
        } else {
            skipped += 1;
        }
    }
    Ok(CopySummary { copied, skipped })
}

// ---------------------------------------------------------------------------
// Claude Code (CLI) profiles + history sharing
// ---------------------------------------------------------------------------

const CODE_PROJECTS_DIR: &str = "projects";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeInstall {
    pub id: String,
    pub name: String,
    /// "default" for the implicit ~/.claude install, "profile" for managed ones.
    pub kind: String,
    pub config_dir: String,
    pub alias_name: Option<String>,
    pub managed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeProject {
    /// On-disk folder name under `<config>/projects/`, e.g. `-Users-foo-bar`.
    pub id: String,
    /// Best-effort decoded path. Ambiguous when project name contains `-`,
    /// so the UI should treat it as a hint, not ground truth.
    pub display_path: String,
    pub session_count: u32,
    pub total_bytes: u64,
    pub last_modified_ms: i64,
    /// First user prompt of the most-recent session, truncated to 240 chars.
    pub first_message_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairCodeProjectShare {
    pub id: String,
    pub display_path: String,
    pub source_present: bool,
    pub target_present: bool,
    pub source_session_count: u32,
    pub target_session_count: u32,
    pub source_bytes: u64,
    pub target_bytes: u64,
    pub source_last_modified_ms: i64,
    pub target_last_modified_ms: i64,
    /// True iff the target project dir is a symlink pointing at source.
    pub shared: bool,
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PairCodeShareChange {
    pub project_id: String,
    pub shared: bool,
}

fn default_code_config_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".claude"))
}

fn code_install_from_default() -> Result<Option<CodeInstall>, String> {
    let dir = default_code_config_dir()?;
    if !dir.is_dir() {
        return Ok(None);
    }
    Ok(Some(CodeInstall {
        id: "default".to_string(),
        name: "Default".to_string(),
        kind: "default".to_string(),
        config_dir: dir.to_string_lossy().to_string(),
        alias_name: None,
        managed: false,
    }))
}

fn code_install_from_profile(profile: &RegistryProfile) -> Option<CodeInstall> {
    // The CLI persists Code profiles as { configDir, aliasName }. A profile may
    // have a Desktop/Codex side but NO Claude Code dir yet (code: null) — we
    // still surface it as a column at the conventional ~/.claude-<slug> path so
    // the user sees ALL their real profiles. The dir is created on first
    // share/edit into it; until then its content simply reads as absent.
    let explicit = profile
        .code
        .as_ref()
        .and_then(|c| c.get("configDir"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let alias_name = profile
        .code
        .as_ref()
        .and_then(|c| c.get("aliasName"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let clean = sanitize_profile_name(&profile.name);
    let config_dir = match explicit {
        Some(dir) => dir,
        None => home_dir()
            .ok()?
            .join(format!(".claude-{clean}"))
            .to_string_lossy()
            .to_string(),
    };
    Some(CodeInstall {
        id: format!("profile:{clean}"),
        name: profile.name.clone(),
        kind: "profile".to_string(),
        config_dir,
        alias_name,
        managed: true,
    })
}

/// Subdirectories under `~/.claude` we never seed because they carry
/// chat history (= account data) or per-shell ephemera.
const CODE_SEED_EXCLUDE: &[&str] = &["projects", "shell-snapshots", "todos", "statsig"];

/// Marker comments framing the managed-alias block we append to the user's
/// shell rc file. Re-running `create_code_profile` for an existing name
/// rewrites the contents of this block in place.
const ALIAS_MARK_BEGIN: &str = "# >>> claude-multiprofile managed (do not edit)";
const ALIAS_MARK_END: &str = "# <<< claude-multiprofile managed";

fn copy_seed_dir(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| format!("Create {}: {e}", target.display()))?;
    for entry in fs::read_dir(source).map_err(|e| format!("Read {}: {e}", source.display()))? {
        let entry = entry.map_err(|e| format!("Read seed entry: {e}"))?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if CODE_SEED_EXCLUDE.iter().any(|n| name_str.as_ref() == *n) {
            continue;
        }
        if name_str.contains("credential") || name_str.starts_with(".credentials") {
            continue;
        }
        let src_path = entry.path();
        let dst_path = target.join(&name);
        let ty = entry
            .file_type()
            .map_err(|e| format!("Read file type: {e}"))?;
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ty.is_file() {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("Copy {}: {e}", src_path.display()))?;
        }
    }
    Ok(())
}

/// Append (or replace) a "managed" alias block in the user's zshrc.
/// Returns the rc-file path written. Only handles zsh — bash/fish can
/// be layered on later if anyone asks.
fn write_zsh_alias_block(alias_name: &str, config_dir: &Path) -> Result<PathBuf, String> {
    let home = home_dir()?;
    let rc = home.join(".zshrc");
    let existing = fs::read_to_string(&rc).unwrap_or_default();

    let alias_line = format!(
        "alias {alias_name}='CLAUDE_CONFIG_DIR={} claude'",
        shell_quote_single(config_dir)
    );
    let block = format!(
        "{ALIAS_MARK_BEGIN}\n{}\n{ALIAS_MARK_END}\n",
        alias_line
    );

    let new_contents = if let (Some(start), Some(end)) =
        (existing.find(ALIAS_MARK_BEGIN), existing.find(ALIAS_MARK_END))
    {
        let end_line_end = existing[end..]
            .find('\n')
            .map(|p| end + p + 1)
            .unwrap_or(existing.len());
        let mut out = String::with_capacity(existing.len() + block.len());
        out.push_str(&existing[..start]);
        out.push_str(&block);
        out.push_str(&existing[end_line_end..]);
        out
    } else {
        let mut out = existing.clone();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&block);
        out
    };

    fs::write(&rc, new_contents).map_err(|e| format!("Write {}: {e}", rc.display()))?;
    Ok(rc)
}

pub fn create_code_profile(
    name: String,
    seed_from_default: bool,
) -> Result<CodeInstall, String> {
    let clean = sanitize_profile_name(&name);
    if clean.is_empty() {
        return Err("Profile name cannot be empty".to_string());
    }
    if clean == "claude" {
        return Err("Alias would shadow the bare `claude` command".to_string());
    }

    let home = home_dir()?;
    let config_dir = home.join(format!(".claude-{clean}"));
    let default_dir = default_code_config_dir()?;
    if config_dir == default_dir {
        return Err("Refusing to use the default ~/.claude directory".to_string());
    }

    let alias_name = format!("claude-{clean}");

    let mut registry = load_registry()?;
    // If a profile with this name already exists, UPDATE it (e.g. add Code
    // to an existing Desktop entry). Reject if it already has Code.
    let existing_idx = registry.profiles.iter().position(|p| p.name == clean);
    if let Some(i) = existing_idx {
        if registry.profiles[i].code.is_some() {
            return Err(format!(
                "Code profile \"{clean}\" already exists — pick a different name"
            ));
        }
    }

    if config_dir.exists() {
        return Err(format!(
            "Config dir {} already exists — pick a different name",
            config_dir.display()
        ));
    }
    fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Create {}: {e}", config_dir.display()))?;

    if seed_from_default && default_dir.exists() {
        copy_seed_dir(&default_dir, &config_dir)?;
    }

    let rc_path = write_zsh_alias_block(&alias_name, &config_dir)?;

    let code_json = serde_json::json!({
        "configDir": config_dir.to_string_lossy(),
        "aliasName": alias_name,
        "shell": "zsh",
        "rcPath": rc_path.to_string_lossy(),
    });

    match existing_idx {
        Some(i) => {
            registry.profiles[i].code = Some(code_json);
            registry.profiles[i].profile_type = "both".to_string();
        }
        None => {
            registry.profiles.push(RegistryProfile {
                name: clean.clone(),
                profile_type: "code".to_string(),
                desktop: None,
                code: Some(code_json),
                codex: None,
                created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            });
        }
    }
    save_registry(&registry)?;

    Ok(CodeInstall {
        id: format!("profile:{clean}"),
        name: clean,
        kind: "profile".to_string(),
        config_dir: config_dir.to_string_lossy().to_string(),
        alias_name: Some(alias_name),
        managed: true,
    })
}

pub fn list_code_installs() -> Result<Vec<CodeInstall>, String> {
    let mut installs = Vec::new();
    if let Some(default) = code_install_from_default()? {
        installs.push(default);
    }
    let registry = load_registry()?;
    for profile in &registry.profiles {
        if let Some(install) = code_install_from_profile(profile) {
            installs.push(install);
        }
    }
    // Auto-discover ~/.claude-<name> config dirs that exist on disk but aren't in
    // the registry (e.g. accounts the user runs via CLAUDE_CONFIG_DIR by hand).
    // Without this, those real accounts never appear as matrix columns, so the
    // grid collapses to a single column and sharing has nothing to share between.
    installs.extend(discover_disk_code_installs(&installs));
    Ok(installs)
}

/// A Claude Code config dir is "real" (vs a blank leftover) if it has recorded
/// at least one project or some command history.
fn code_dir_has_content(dir: &Path) -> bool {
    let has_projects = fs::read_dir(dir.join("projects"))
        .map(|mut r| r.next().is_some())
        .unwrap_or(false);
    let has_history = fs::metadata(dir.join("history.jsonl"))
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    has_projects || has_history
}

/// Find `~/.claude-<name>` directories on disk that look like real Claude Code
/// config dirs and aren't already covered by the default/registry installs.
fn discover_disk_code_installs(existing: &[CodeInstall]) -> Vec<CodeInstall> {
    let home = match home_dir() {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let existing_dirs: std::collections::HashSet<PathBuf> = existing
        .iter()
        .map(|i| {
            let p = PathBuf::from(&i.config_dir);
            fs::canonicalize(&p).unwrap_or(p)
        })
        .collect();
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(&home) else {
        return out;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(suffix) = name.strip_prefix(".claude-") else {
            continue;
        };
        if suffix.is_empty() {
            continue;
        }
        // Must look like a real Claude Code config dir, not some unrelated folder.
        if !p.join("settings.json").is_file()
            && !p.join("sessions").is_dir()
            && !p.join("projects").is_dir()
        {
            continue;
        }
        // Skip BLANK config dirs (no sessions ever recorded) — these are usually
        // leftovers from deleted profiles and would clutter the matrix with empty
        // columns. A real account has at least one project or some history.
        if !code_dir_has_content(&p) {
            continue;
        }
        let canon = fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if existing_dirs.contains(&canon) {
            continue;
        }
        out.push(CodeInstall {
            id: format!("disk:{suffix}"),
            name: suffix.to_string(),
            kind: "profile".to_string(),
            config_dir: p.to_string_lossy().to_string(),
            alias_name: None,
            managed: false,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ===========================================================================
// Codex profiles (Desktop launcher) — IDENTICAL in principle to Claude
// Desktop. The Codex desktop app is Chromium-based (Electron) and honors
// --user-data-dir, which isolates the *web* layer (cookies, local storage) in
// ~/Library/Application Support/Codex-<Name>. BUT Codex's OAuth token lives in
// the agent home, NOT the web profile, so a fully-isolated "Codex profile"
// needs THREE things:
//   1. its own --user-data-dir  (web layer)
//   2. its own CODEX_HOME       (agent home: auth.json, config.toml, sessions)
//   3. cli_auth_credentials_store="file" in that CODEX_HOME's config.toml
// (3) is the subtle one: Codex defaults the token store to "auto", which writes
// to a GLOBAL macOS keychain item ("Codex Auth") keyed by app identity, NOT by
// CODEX_HOME — so without pinning "file" two profiles share one login the
// moment the keyring write succeeds. We pass CODEX_HOME via `open --env` and
// seed config.toml with the file backend (see ensure_codex_file_auth_backend).
// The default install keeps CODEX_HOME unset → ~/.codex and is left untouched.
// (Note: the Electron safeStorage master key that encrypts each web store is
// also global per app install; that doesn't break isolation — the cookie jars
// themselves are still separated by --user-data-dir.)
// ===========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodexInstall {
    pub id: String,
    pub name: String,
    /// "default" for the implicit Codex.app install, "profile" for managed ones.
    pub kind: String,
    pub data_dir: String,
    pub app_path: Option<String>,
    pub launcher_path: Option<String>,
    pub managed: bool,
    pub is_running: bool,
}

fn find_codex_app() -> Result<Option<PathBuf>, String> {
    let candidates = [
        PathBuf::from("/Applications/Codex.app"),
        home_dir()?.join("Applications").join("Codex.app"),
    ];
    Ok(candidates.into_iter().find(|path| path.exists()))
}

fn default_codex_desktop_data_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Library")
        .join("Application Support")
        .join("Codex"))
}

fn codex_data_dir_for(name: &str) -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Library")
        .join("Application Support")
        .join(format!("Codex-{}", title_case(name))))
}

fn codex_launcher_path_for(name: &str) -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Applications")
        .join(format!("Codex {}.app", title_case(name))))
}

/// Per-profile CODEX_HOME (its own auth.json / config.toml / sessions), mirroring
/// Claude Code's `~/.claude-<name>` convention. The default install uses the
/// unsuffixed `~/.codex`.
fn codex_home_dir_for(name: &str) -> Result<PathBuf, String> {
    let clean = sanitize_profile_name(name);
    Ok(home_dir()?.join(format!(".codex-{clean}")))
}

/// Pin a profile's auth token to the FILE backend ($CODEX_HOME/auth.json).
///
/// Codex defaults `cli_auth_credentials_store = "auto"`, which writes the OAuth
/// token to a GLOBAL macOS keychain item (service "Codex Auth"), keyed by app
/// identity — NOT by CODEX_HOME. So `auto` would let two profiles share one
/// login the moment the keyring write succeeds, defeating CODEX_HOME isolation.
/// Forcing `"file"` keeps the token inside the per-profile CODEX_HOME, which is
/// the only thing CODEX_HOME actually scopes. Idempotent; preserves the rest of
/// config.toml. Never called for the default ~/.codex.
fn ensure_codex_file_auth_backend(codex_home: &Path) -> Result<(), String> {
    let path = codex_home.join("config.toml");
    let raw = fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("Parse {}: {e}", path.display()))?;
    if doc
        .get("cli_auth_credentials_store")
        .and_then(|v| v.as_str())
        == Some("file")
    {
        return Ok(());
    }
    doc["cli_auth_credentials_store"] = toml_edit::value("file");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create {}: {e}", parent.display()))?;
    }
    write_string_atomically(&path, &doc.to_string())
}

fn codex_install_from_default() -> Result<Option<CodexInstall>, String> {
    let Some(app_path) = find_codex_app()? else {
        return Ok(None);
    };
    let data_dir = default_codex_desktop_data_dir()?;
    if !data_dir.exists() {
        return Ok(None);
    }
    Ok(Some(CodexInstall {
        id: "default".to_string(),
        name: "default".to_string(),
        kind: "default".to_string(),
        data_dir: data_dir.to_string_lossy().to_string(),
        app_path: Some(app_path.to_string_lossy().to_string()),
        launcher_path: None,
        managed: false,
        is_running: false,
    }))
}

fn codex_install_from_profile(profile: &RegistryProfile) -> Option<CodexInstall> {
    let codex = profile.codex.as_ref()?;
    let data_dir = codex.get("dataDir").and_then(|v| v.as_str())?.to_string();
    let launcher_path = codex
        .get("launcherPath")
        .or_else(|| codex.get("appPath"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let app_path = codex
        .get("codexAppPath")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(CodexInstall {
        id: format!("profile:{}", profile.name),
        name: profile.name.clone(),
        kind: "profile".to_string(),
        data_dir,
        app_path,
        launcher_path,
        managed: true,
        is_running: false,
    })
}

/// Codex.app launched with `--user-data-dir=<path>` => that profile is live;
/// without the flag => the default install. Mirrors detect_running_data_dirs.
/// Extract a `--user-data-dir=<path>` value from a process argv string, or None.
/// Splits at the next ` --` so a data dir containing spaces survives intact.
fn parse_user_data_dir(args: &str) -> Option<PathBuf> {
    let idx = args.find("--user-data-dir=")?;
    let after = &args[idx + "--user-data-dir=".len()..];
    let path_str = after
        .find(" --")
        .map(|j| &after[..j])
        .unwrap_or(after)
        .trim()
        .trim_end_matches('\0');
    (!path_str.is_empty()).then(|| PathBuf::from(path_str))
}

/// From `lsof -Fn` output, return the first open file's top-level data dir under
/// `<base>/Codex` or `<base>/Codex-<name>`. Excludes unrelated apps (e.g.
/// `CodexBar`). Pure + testable given the raw output and the support-dir base.
fn codex_data_dir_from_lsof(raw: &str, base: &Path) -> Option<PathBuf> {
    let prefix = format!("{}/", base.to_string_lossy());
    for line in raw.lines() {
        let Some(path) = line.strip_prefix('n') else {
            continue;
        };
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        let seg = rel.split('/').next().unwrap_or("");
        if seg == "Codex" || seg.starts_with("Codex-") {
            return Some(base.join(seg));
        }
    }
    None
}

/// Resolve a Codex pid's open desktop data dir via a single bounded lsof. Used
/// ONLY for processes whose argv lacks --user-data-dir, so the default is never
/// a catch-all for every flagless Codex process.
fn lsof_codex_data_dir(pid: &str) -> Option<PathBuf> {
    let base = home_dir().ok()?.join("Library/Application Support");
    let out = Command::new("/usr/sbin/lsof")
        .args(["-p", pid, "-Fn"])
        .output()
        .ok()?;
    // lsof commonly exits nonzero with partial output — scan whatever we got.
    let raw = String::from_utf8_lossy(&out.stdout);
    codex_data_dir_from_lsof(&raw, &base)
}

fn detect_running_codex_data_dirs() -> Vec<PathBuf> {
    let out = match Command::new("/bin/ps").args(["-Aww", "-o", "pid=,args="]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let raw = String::from_utf8_lossy(&out.stdout);
    let mut running: Vec<PathBuf> = Vec::new();
    for line in raw.lines() {
        let line = line.trim_start();
        let Some((pid, args)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let args = args.trim_start();
        if !args.contains("/Codex.app/Contents/MacOS/Codex") {
            continue;
        }
        if args.contains("Helper")
            || args.contains("Renderer")
            || args.contains("Crashpad")
            || args.contains("GPU")
            || args.contains("Utility")
        {
            continue;
        }
        // argv fast path (every in-app + launcher-.app launch carries the flag);
        // else resolve the real open data dir via lsof; else attribute to NOTHING
        // — never fold a flagless Codex process onto the default (the old bug
        // that left the default stuck-"live" and profiles invisible).
        if let Some(p) = parse_user_data_dir(args) {
            running.push(p);
        } else if let Some(p) = lsof_codex_data_dir(pid.trim()) {
            running.push(p);
        }
    }
    running
}

/// Default Codex.app install + every Codex profile in the registry, tagged
/// with is_running.
pub fn list_codex_installs() -> Result<Vec<CodexInstall>, String> {
    let mut installs = Vec::new();
    if let Some(default) = codex_install_from_default()? {
        installs.push(default);
    }
    let registry = load_registry()?;
    for profile in &registry.profiles {
        if let Some(install) = codex_install_from_profile(profile) {
            installs.push(install);
        }
    }
    let running_paths = detect_running_codex_data_dirs();
    let running_canon: Vec<PathBuf> = running_paths
        .iter()
        .filter_map(|p| fs::canonicalize(p).ok().or_else(|| Some(p.clone())))
        .collect();
    for install in &mut installs {
        let mine_raw = PathBuf::from(&install.data_dir);
        let mine_canon = fs::canonicalize(&mine_raw).unwrap_or(mine_raw);
        install.is_running = running_canon.iter().any(|p| p == &mine_canon);
    }
    Ok(installs)
}

/// Create a Codex Desktop profile: a fresh user-data-dir + a launcher .app
/// that opens Codex against it. Same machinery as create_desktop_profile.
pub fn create_codex_profile(name: String) -> Result<CodexInstall, String> {
    let clean_name = sanitize_profile_name(&name);
    if clean_name.is_empty() {
        return Err("Profile name cannot be empty".to_string());
    }

    let codex_app_path = find_codex_app()?
        .ok_or_else(|| "Codex.app was not found in /Applications or ~/Applications".to_string())?;
    let data_dir = codex_data_dir_for(&clean_name)?;
    if data_dir == default_codex_desktop_data_dir()? {
        return Err("Refusing to use the default Codex data directory".to_string());
    }
    let codex_home = codex_home_dir_for(&clean_name)?;
    if codex_home == codex_home_dir()? {
        return Err("Refusing to use the default ~/.codex directory".to_string());
    }
    let launcher_path = codex_launcher_path_for(&clean_name)?;

    let mut registry = load_registry()?;
    let existing_idx = registry.profiles.iter().position(|p| p.name == clean_name);
    if let Some(i) = existing_idx {
        if registry.profiles[i].codex.is_some() {
            return Err(format!("Codex profile \"{clean_name}\" already exists"));
        }
    }

    fs::create_dir_all(&data_dir).map_err(|e| format!("Create Codex profile data dir: {e}"))?;
    // A fresh, empty CODEX_HOME forces a brand-new login on first launch.
    fs::create_dir_all(&codex_home).map_err(|e| format!("Create Codex profile home: {e}"))?;
    // Pin the file auth backend so the new login can't land in the shared keychain.
    ensure_codex_file_auth_backend(&codex_home)?;
    compile_launcher_app(
        &clean_name,
        &data_dir,
        &launcher_path,
        &codex_app_path,
        Some(&codex_home),
    )?;

    let codex_json = serde_json::json!({
        "dataDir": data_dir.to_string_lossy(),
        "codexHome": codex_home.to_string_lossy(),
        "launcherPath": launcher_path.to_string_lossy(),
        "codexAppPath": codex_app_path.to_string_lossy(),
    });
    match existing_idx {
        Some(i) => {
            registry.profiles[i].codex = Some(codex_json);
            if registry.profiles[i].profile_type != "both" {
                registry.profiles[i].profile_type = "both".to_string();
            }
        }
        None => {
            registry.profiles.push(RegistryProfile {
                name: clean_name.clone(),
                profile_type: "codex".to_string(),
                desktop: None,
                code: None,
                codex: Some(codex_json),
                created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            });
        }
    }
    save_registry(&registry)?;

    Ok(CodexInstall {
        id: format!("profile:{clean_name}"),
        name: clean_name,
        kind: "profile".to_string(),
        data_dir: data_dir.to_string_lossy().to_string(),
        app_path: Some(codex_app_path.to_string_lossy().to_string()),
        launcher_path: Some(launcher_path.to_string_lossy().to_string()),
        managed: true,
        is_running: false,
    })
}

pub fn launch_codex_install(install_id: String) -> Result<(), String> {
    let install = list_codex_installs()?
        .into_iter()
        .find(|i| i.id == install_id)
        .ok_or_else(|| format!("Codex install not found: {install_id}"))?;

    if install.kind == "default" {
        let app = install
            .app_path
            .ok_or_else(|| "Default Codex app path is missing".to_string())?;
        let mut open = Command::new("/usr/bin/open");
        open.arg(app);
        return run_command(open, "Launch Codex");
    }

    // Managed profile: isolate BOTH layers. Launch the real Codex.app directly
    // with `--env CODEX_HOME=…` (the in-app button is always correct this way,
    // even for profiles whose launcher .app predates this fix) plus the
    // Chromium `--user-data-dir`. We don't go through the launcher .app here so
    // a stale launcher can't reintroduce the shared-account bug.
    let app = install
        .app_path
        .clone()
        .ok_or_else(|| "Codex app path is missing".to_string())?;
    let codex_home = codex_home_dir_for(&install.name)?;
    fs::create_dir_all(&codex_home).map_err(|e| format!("Create Codex profile home: {e}"))?;
    // Guarantee the file auth backend before every launch — this also heals a
    // legacy profile that logged in under the old (shared-keychain) default.
    ensure_codex_file_auth_backend(&codex_home)?;
    heal_codex_launcher(&install, &codex_home);

    // Pass `--user-data-dir=<dir>` (= form, single token) so the running
    // process argv matches detect_running_codex_data_dirs' `--user-data-dir=`
    // needle — otherwise an in-app-launched profile never reports as running.
    let mut open = Command::new("/usr/bin/open");
    open.args([
        "-n",
        "-a",
        &app,
        "--env",
        &format!("CODEX_HOME={}", codex_home.display()),
        "--args",
        &format!("--user-data-dir={}", install.data_dir),
    ]);
    run_command(open, "Launch Codex profile")
}

/// Bring a pre-CODEX_HOME launcher .app up to date: recompile it to embed the
/// profile's CODEX_HOME and record `codexHome` in the registry. No-op once the
/// profile is already isolated (codexHome present). Best-effort — failures here
/// don't block the in-app launch, which already passes CODEX_HOME directly.
fn heal_codex_launcher(install: &CodexInstall, codex_home: &Path) {
    let (Some(launcher), Some(app)) =
        (install.launcher_path.as_ref(), install.app_path.as_ref())
    else {
        return;
    };
    let mut registry = match load_registry() {
        Ok(r) => r,
        Err(_) => return,
    };
    let already = match registry
        .profiles
        .iter()
        .find(|p| p.name == install.name)
        .and_then(|p| p.codex.as_ref())
    {
        Some(codex) => codex.get("codexHome").and_then(|v| v.as_str()).is_some(),
        None => true, // no codex entry to heal
    };
    if already {
        return;
    }
    let data_dir = PathBuf::from(&install.data_dir);
    if compile_launcher_app(
        &install.name,
        &data_dir,
        Path::new(launcher),
        Path::new(app),
        Some(codex_home),
    )
    .is_err()
    {
        return;
    }
    if let Some(codex) = registry
        .profiles
        .iter_mut()
        .find(|p| p.name == install.name)
        .and_then(|p| p.codex.as_mut())
        .and_then(|c| c.as_object_mut())
    {
        codex.insert(
            "codexHome".into(),
            serde_json::Value::String(codex_home.to_string_lossy().to_string()),
        );
    }
    let _ = save_registry(&registry);
}

// ===========================================================================
// Deleting profiles
// ===========================================================================

/// Recompute a profile's `type` from which halves remain after a deletion.
fn profile_type_for(p: &RegistryProfile) -> String {
    let claude = p.desktop.is_some() || p.code.is_some();
    let codex = p.codex.is_some();
    match (claude, codex) {
        (true, true) => "both".to_string(),
        (false, true) => "codex".to_string(),
        (true, false) => {
            if p.desktop.is_some() && p.code.is_some() {
                "both".to_string()
            } else if p.desktop.is_some() {
                "desktop".to_string()
            } else {
                "code".to_string()
            }
        }
        (false, false) => "empty".to_string(),
    }
}

/// Remove exactly the `alias <alias_name>=...` line from ~/.zshrc (the inverse
/// of write_zsh_alias_block), then drop any now-empty managed marker block.
/// The writer only keeps the most-recently-created alias inside the markers,
/// so we scan for the specific alias line ANYWHERE rather than assuming it
/// lives in the block.
fn remove_zsh_alias_line(alias_name: &str) -> Result<(), String> {
    let home = home_dir()?;
    let rc = home.join(".zshrc");
    let existing = match fs::read_to_string(&rc) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Read {}: {e}", rc.display())),
    };
    let needle = format!("alias {alias_name}=");
    let kept: Vec<&str> = existing
        .lines()
        .filter(|line| !line.trim_start().starts_with(&needle))
        .collect();

    // Drop a now-empty managed block (BEGIN immediately followed by END).
    let mut out: Vec<&str> = Vec::with_capacity(kept.len());
    let mut i = 0;
    while i < kept.len() {
        if kept[i].contains(ALIAS_MARK_BEGIN)
            && i + 1 < kept.len()
            && kept[i + 1].contains(ALIAS_MARK_END)
        {
            i += 2; // skip the empty BEGIN/END pair
            continue;
        }
        out.push(kept[i]);
        i += 1;
    }

    let mut new_contents = out.join("\n");
    if existing.ends_with('\n') && !new_contents.ends_with('\n') {
        new_contents.push('\n');
    }
    fs::write(&rc, new_contents).map_err(|e| format!("Write {}: {e}", rc.display()))?;
    Ok(())
}

/// Delete a Claude profile (Desktop launcher + Code alias). Soft by default:
/// removes the launcher .app, the Code CLI alias, and the registry half(s),
/// but keeps the data dir unless `delete_data` is set.
pub fn delete_desktop_profile(install_id: String, delete_data: bool) -> Result<(), String> {
    let install = list_desktop_installs()?
        .into_iter()
        .find(|i| i.id == install_id)
        .ok_or_else(|| format!("Profile not found: {install_id}"))?;
    if install.is_running {
        return Err(format!("Quit {} before deleting it", install.name));
    }

    // The default install has no launcher / registry entry — the only thing to
    // remove is the real ~/Library data dir (the primary login + chats). Allow
    // it (profiles are equal), but ONLY with an explicit data-erase + a path
    // assertion + the guarded delete.
    if install.kind == "default" {
        if !delete_data {
            return Err(
                "The default install has nothing to remove but its data — enable 'erase data' to delete it."
                    .to_string(),
            );
        }
        let dd = PathBuf::from(&install.data_dir);
        if dd != default_desktop_data_dir()? {
            return Err("Default data dir path mismatch — refusing to delete.".to_string());
        }
        remove_data_dir(&dd)?;
        return Ok(());
    }

    let mut registry = load_registry()?;
    let Some(idx) = registry.profiles.iter().position(|p| p.name == install.name) else {
        return Err(format!("Profile \"{}\" not in registry", install.name));
    };

    // Remove the launcher .app — install.launcher_path, NEVER app_path
    // (app_path is the real /Applications/Claude.app).
    if let Some(launcher) = &install.launcher_path {
        remove_path(Path::new(launcher))?;
    }

    // Strip the Code CLI alias + optionally delete the Code config dir.
    if let Some(code) = registry.profiles[idx].code.clone() {
        if let Some(alias) = code.get("aliasName").and_then(|v| v.as_str()) {
            let _ = remove_zsh_alias_line(alias);
        }
        if delete_data {
            if let Some(cfg) = code.get("configDir").and_then(|v| v.as_str()) {
                remove_data_dir(Path::new(cfg))?;
            }
        }
    }

    if delete_data {
        // Assert the deterministic path, not the trusted registry string — a
        // corrupted profiles.json can't redirect the erase elsewhere.
        let dd = PathBuf::from(&install.data_dir);
        if dd != default_data_dir_for(&install.name)? {
            return Err("Profile data dir path mismatch — refusing to delete.".to_string());
        }
        remove_data_dir(&dd)?;
    }

    // Clear only the Claude halves; keep a Codex half if this entry has one.
    registry.profiles[idx].desktop = None;
    registry.profiles[idx].code = None;
    let new_type = profile_type_for(&registry.profiles[idx]);
    if new_type == "empty" {
        registry.profiles.remove(idx);
    } else {
        registry.profiles[idx].profile_type = new_type;
    }
    save_registry(&registry)?;
    Ok(())
}

/// Delete a Codex profile (Desktop launcher). Soft by default.
pub fn delete_codex_profile(install_id: String, delete_data: bool) -> Result<(), String> {
    let install = list_codex_installs()?
        .into_iter()
        .find(|i| i.id == install_id)
        .ok_or_else(|| format!("Codex profile not found: {install_id}"))?;
    if install.is_running {
        return Err(format!("Quit Codex {} before deleting it", install.name));
    }

    // Default Codex install: only its real data dir is removable, and ONLY when
    // it's the sole Codex install — ~/.codex (its CODEX_HOME) is shared by every
    // Codex profile, so erasing it while others exist would nuke their auth too.
    if install.kind == "default" {
        if !delete_data {
            return Err(
                "The default Codex install has nothing to remove but its data — enable 'erase data' to delete it."
                    .to_string(),
            );
        }
        if codex_profile_columns()?.len() > 1 {
            return Err(
                "~/.codex is shared by all Codex profiles — delete the other Codex profiles before erasing the default."
                    .to_string(),
            );
        }
        let dd = PathBuf::from(&install.data_dir);
        if dd != default_codex_desktop_data_dir()? {
            return Err("Default Codex data dir path mismatch — refusing to delete.".to_string());
        }
        remove_data_dir(&dd)?;
        // Also the shared ~/.codex agent home (sole install, so safe).
        remove_data_dir(&codex_home_dir()?)?;
        return Ok(());
    }

    let mut registry = load_registry()?;
    let Some(idx) = registry.profiles.iter().position(|p| p.name == install.name) else {
        return Err(format!("Codex profile \"{}\" not in registry", install.name));
    };

    if let Some(launcher) = &install.launcher_path {
        remove_path(Path::new(launcher))?;
    }
    if delete_data {
        // Assert the deterministic Chromium data dir, not the registry string.
        let dd = PathBuf::from(&install.data_dir);
        if dd != codex_data_dir_for(&install.name)? {
            return Err("Codex profile data dir path mismatch — refusing to delete.".to_string());
        }
        remove_data_dir(&dd)?;
        // Also erase the per-profile CODEX_HOME (auth.json, sessions, config).
        // Use the DETERMINISTIC path (~/.codex-<clean>), never a registry-
        // supplied string. Belt-and-suspenders: also refuse the shared default.
        let codex_home = codex_home_dir_for(&install.name)?;
        if codex_home_dir().map(|d| d != codex_home).unwrap_or(true) {
            remove_data_dir(&codex_home)?;
        }
    }

    registry.profiles[idx].codex = None;
    let new_type = profile_type_for(&registry.profiles[idx]);
    if new_type == "empty" {
        registry.profiles.remove(idx);
    } else {
        registry.profiles[idx].profile_type = new_type;
    }
    save_registry(&registry)?;
    Ok(())
}

/// Best-effort: replace every `-` with `/`. Original `-` in dir names is lost,
/// so we mark the result as a hint by returning the encoded form too.
fn decode_project_dir_name(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    name.replace('-', "/")
}

fn safe_project_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains('/')
        && !id.contains('\\')
        && id != "."
        && id != ".."
        && !id.split('.').any(|part| part == "..")
}

fn system_time_to_epoch_ms(time: std::time::SystemTime) -> i64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Read the first JSONL line and try to extract a user-visible string.
/// Tolerant of multiple shapes — Claude Code has evolved over time, so we
/// accept either `content` (queue-operation) or `message.content`.
fn read_first_user_message(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(s) = value.get("content").and_then(|v| v.as_str()) {
            return Some(truncate_preview(s));
        }
        if let Some(msg) = value.get("message") {
            if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
                return Some(truncate_preview(s));
            }
            if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
                for part in arr {
                    if let Some(s) = part.get("text").and_then(|v| v.as_str()) {
                        return Some(truncate_preview(s));
                    }
                }
            }
        }
        // Skip non-user records and keep scanning until we see something useful.
    }
    None
}

fn truncate_preview(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= 240 {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(240).collect();
    out.push('…');
    out
}

/// Turn a first-message preview into a tidy one-line session title: collapse all
/// whitespace/newlines into single spaces and cap at ~80 chars, so the session
/// list shows a readable title rather than a multi-line blob or a bare UUID.
fn title_from_preview(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() <= 80 {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(80).collect();
    out.push('…');
    out
}

/// A first-message text that is injected context / tooling boilerplate, not the
/// user's real prompt — skipped when picking a session title.
fn is_noise_title(t: &str) -> bool {
    let s = t.trim_start();
    s.is_empty()
        || s.starts_with('<') // <environment_context> / <system-reminder> / <command-name> / <permissions…>
        || s.starts_with("Caveat:")
        || s.starts_with("Conversation compacted")
        || s.starts_with("[Request interrupted")
        || s.starts_with("This session is being continued")
}

/// Pull the user-authored text blocks out of one Claude transcript record
/// (content as a string, or message.content as string / array-of-{text}).
fn claude_record_user_texts(v: &serde_json::Value) -> Vec<String> {
    let is_user = v.get("type").and_then(|t| t.as_str()) == Some("user")
        || v.get("message")
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            == Some("user");
    if !is_user {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(s) = v.get("content").and_then(|c| c.as_str()) {
        out.push(s.to_string());
    }
    if let Some(m) = v.get("message") {
        if let Some(s) = m.get("content").and_then(|c| c.as_str()) {
            out.push(s.to_string());
        }
        if let Some(arr) = m.get("content").and_then(|c| c.as_array()) {
            for part in arr {
                if let Some(s) = part.get("text").and_then(|x| x.as_str()) {
                    out.push(s.to_string());
                }
            }
        }
    }
    out
}

/// Codex variant of `claude_record_user_texts`: a `response_item` payload with
/// role "user" carries content parts `{ text }`.
fn codex_record_user_texts(v: &serde_json::Value) -> Vec<String> {
    let payload = match v.get("payload") {
        Some(p) => p,
        None => return Vec::new(),
    };
    if payload.get("role").and_then(|r| r.as_str()) != Some("user") {
        return Vec::new();
    }
    payload
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|part| part.get("text").and_then(|x| x.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Collect user-authored text snippets from the HEAD of a transcript (bounded so
/// a giant file is never fully loaded; the opening prompt is always near the top).
fn session_head_user_texts(
    path: &Path,
    extract: impl Fn(&serde_json::Value) -> Vec<String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(f) = fs::File::open(path) else {
        return out;
    };
    let mut reader = std::io::BufReader::new(f);
    let mut line = String::new();
    for _ in 0..80 {
        line.clear();
        match std::io::BufRead::read_line(&mut reader, &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
            out.extend(extract(&v));
            if out.len() >= 12 {
                break;
            }
        }
    }
    out
}

/// Choose a session title: the first *real* user prompt; failing that, the first
/// non-XML text (e.g. a continuation summary) so continued/compacted sessions get
/// a readable label rather than the bare UUID. Never an `<…>` injected block.
fn pick_title(texts: &[String]) -> Option<String> {
    texts
        .iter()
        .find(|t| !is_noise_title(t))
        .or_else(|| {
            texts
                .iter()
                .find(|t| !t.trim_start().starts_with('<') && !t.trim().is_empty())
        })
        .map(|t| title_from_preview(t))
}

fn read_claude_session_title(path: &Path) -> Option<String> {
    pick_title(&session_head_user_texts(path, claude_record_user_texts))
}

fn read_codex_session_title(path: &Path) -> Option<String> {
    pick_title(&session_head_user_texts(path, codex_record_user_texts))
}

#[derive(Debug, Default)]
struct ProjectFolderStats {
    session_count: u32,
    total_bytes: u64,
    last_modified_ms: i64,
    most_recent_session: Option<PathBuf>,
}

fn scan_project_folder(folder: &Path) -> Result<ProjectFolderStats, String> {
    let mut stats = ProjectFolderStats::default();
    let read = match fs::read_dir(folder) {
        Ok(r) => r,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(stats),
        Err(e) => return Err(format!("Read project folder {}: {e}", folder.display())),
    };
    let mut newest_time = std::time::SystemTime::UNIX_EPOCH;
    for entry in read {
        let entry = entry.map_err(|e| format!("Read entry: {e}"))?;
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let is_jsonl = path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"));
        if !is_jsonl {
            continue;
        }
        stats.session_count += 1;
        stats.total_bytes = stats.total_bytes.saturating_add(meta.len());
        if let Ok(modified) = meta.modified() {
            if modified > newest_time {
                newest_time = modified;
                stats.most_recent_session = Some(path.clone());
            }
        }
    }
    stats.last_modified_ms = if stats.session_count == 0 {
        0
    } else {
        system_time_to_epoch_ms(newest_time)
    };
    Ok(stats)
}

pub fn list_code_history(config_dir: &Path) -> Result<Vec<CodeProject>, String> {
    let projects_root = config_dir.join(CODE_PROJECTS_DIR);
    if !projects_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut projects = Vec::new();
    for entry in fs::read_dir(&projects_root)
        .map_err(|e| format!("Read projects root: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Read projects entry: {e}"))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| format!("Read project type: {e}"))?;
        // Skip the `-` placeholder dir Claude Code emits when no cwd is known.
        if !file_type.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if !safe_project_id(&id) || id == "-" {
            continue;
        }
        let stats = scan_project_folder(&path)?;
        let preview = stats
            .most_recent_session
            .as_deref()
            .and_then(read_first_user_message);
        // The `~/.claude/projects/<id>` dir name is a LOSSY encoding of the cwd
        // (both `/` and `.` collapse to `-`), so decoding it gives a garbled path
        // ("乱码"). Read the REAL cwd from the session transcript instead; only
        // fall back to the lossy decode when there's no usable transcript.
        let display_path =
            project_cwd_from_sessions(&path).unwrap_or_else(|| decode_project_dir_name(&id));
        projects.push(CodeProject {
            display_path,
            id,
            session_count: stats.session_count,
            total_bytes: stats.total_bytes,
            last_modified_ms: stats.last_modified_ms,
            first_message_preview: preview,
        });
    }

    // Most recently active first; ties fall back to alphabetical for determinism.
    projects.sort_by(|a, b| {
        b.last_modified_ms
            .cmp(&a.last_modified_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(projects)
}

fn code_project_path(config_dir: &Path, project_id: &str) -> PathBuf {
    config_dir.join(CODE_PROJECTS_DIR).join(project_id)
}

fn share_code_project_one_way(
    source_config: &Path,
    target_config: &Path,
    project_id: &str,
) -> Result<(), String> {
    if !safe_project_id(project_id) {
        return Err(format!("Invalid project id: {project_id}"));
    }
    let source_project = code_project_path(source_config, project_id);
    let target_project = code_project_path(target_config, project_id);
    if !source_project.is_dir() {
        return Err(format!("Project not found in source: {project_id}"));
    }

    let target_root = target_config.join(CODE_PROJECTS_DIR);
    fs::create_dir_all(&target_root).map_err(|e| format!("Create target projects dir: {e}"))?;

    if path_points_to(&target_project, &source_project) {
        return Ok(());
    }
    backup_existing_path(&target_project, target_config, project_id)?;
    symlink_path(&source_project, &target_project)?;
    Ok(())
}

fn make_code_project_independent_one_way(
    source_config: &Path,
    target_config: &Path,
    project_id: &str,
) -> Result<bool, String> {
    let source_project = code_project_path(source_config, project_id);
    let target_project = code_project_path(target_config, project_id);
    if !path_points_to(&target_project, &source_project) {
        return Ok(false);
    }
    remove_path(&target_project)?;
    if source_project.is_dir() {
        copy_dir_recursive(&source_project, &target_project)?;
    }
    Ok(true)
}

fn pair_code_project_share(
    source_config: &Path,
    target_config: &Path,
    project_id: &str,
) -> Result<PairCodeProjectShare, String> {
    let source_path = code_project_path(source_config, project_id);
    let target_path = code_project_path(target_config, project_id);
    let source_meta = fs::symlink_metadata(&source_path).ok();
    let target_meta = fs::symlink_metadata(&target_path).ok();

    let source_present = source_meta
        .as_ref()
        .is_some_and(|m| m.is_dir() || m.file_type().is_symlink());
    let target_present = target_meta
        .as_ref()
        .is_some_and(|m| m.is_dir() || m.file_type().is_symlink());

    let source_stats = if source_present {
        scan_project_folder(&source_path)?
    } else {
        ProjectFolderStats::default()
    };
    let target_stats = if target_present {
        scan_project_folder(&target_path)?
    } else {
        ProjectFolderStats::default()
    };

    let target_to_source = path_points_to(&target_path, &source_path);
    let source_to_target = path_points_to(&source_path, &target_path);
    let direction = if target_to_source {
        "source-to-target"
    } else if source_to_target {
        "target-to-source"
    } else {
        "independent"
    }
    .to_string();

    Ok(PairCodeProjectShare {
        id: project_id.to_string(),
        display_path: decode_project_dir_name(project_id),
        source_present,
        target_present,
        source_session_count: source_stats.session_count,
        target_session_count: target_stats.session_count,
        source_bytes: source_stats.total_bytes,
        target_bytes: target_stats.total_bytes,
        source_last_modified_ms: source_stats.last_modified_ms,
        target_last_modified_ms: target_stats.last_modified_ms,
        shared: target_to_source || source_to_target,
        direction,
    })
}

pub fn list_pair_code_history_shares(
    source_config: &Path,
    target_config: &Path,
) -> Result<Vec<PairCodeProjectShare>, String> {
    let mut ids: BTreeMap<String, ()> = BTreeMap::new();
    for project in list_code_history(source_config)? {
        ids.insert(project.id, ());
    }
    for project in list_code_history(target_config)? {
        ids.insert(project.id, ());
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids.keys() {
        out.push(pair_code_project_share(source_config, target_config, id)?);
    }
    // Sort: shared first, then by source last-modified desc.
    out.sort_by(|a, b| {
        b.shared
            .cmp(&a.shared)
            .then_with(|| b.source_last_modified_ms.cmp(&a.source_last_modified_ms))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(out)
}

fn set_pair_code_project_shared(
    source_config: &Path,
    target_config: &Path,
    project_id: &str,
    desired_shared: bool,
) -> Result<bool, String> {
    let current = pair_code_project_share(source_config, target_config, project_id)?;
    if current.shared == desired_shared {
        return Ok(false);
    }
    if desired_shared {
        share_code_project_one_way(source_config, target_config, project_id)?;
    } else {
        make_code_project_independent_one_way(source_config, target_config, project_id)?;
    }
    Ok(true)
}

pub fn list_pair_code_history_sharing(
    source_config_dir: String,
    target_config_dir: String,
) -> Result<Vec<PairCodeProjectShare>, String> {
    list_pair_code_history_shares(
        Path::new(&source_config_dir),
        Path::new(&target_config_dir),
    )
}

pub fn apply_pair_code_history_sharing(
    source_config_dir: String,
    target_config_dir: String,
    changes: Vec<PairCodeShareChange>,
) -> Result<CopySummary, String> {
    let source = Path::new(&source_config_dir);
    let target = Path::new(&target_config_dir);
    let mut copied = 0;
    let mut skipped = 0;
    for change in changes {
        if set_pair_code_project_shared(source, target, &change.project_id, change.shared)? {
            copied += 1;
        } else {
            skipped += 1;
        }
    }
    Ok(CopySummary { copied, skipped })
}

// ---------------------------------------------------------------------------
// Pair sharing — MCP servers, Cowork Skills, Preferences
// ---------------------------------------------------------------------------
// These three sharing kinds were `ComingSoonPane` placeholders before; this
// block adds the real backend. The design lives in
// docs/plans/2026-05-27-share-redesign.md.
//
// Two sharing models coexist with the existing Extensions/Code-history code:
//
//   Model A — Symlink swap (live share). Unit is a file or directory. Used
//             here for Cowork Skills (per-skill folder under skills-plugin/).
//             Existing helpers (symlink_path, path_points_to, remove_path,
//             backup_existing_path) carry the weight.
//
//   Model B — Copy on apply (one-shot). Unit is a JSON key inside a config
//             file (mcpServers entries, individual preference keys). The
//             helpers below — read_desktop_config, write_json_atomically —
//             do atomic temp-file+rename so we never leave a half-written
//             config behind even if the process is killed mid-write.

const DESKTOP_CONFIG_FILE: &str = "claude_desktop_config.json";
const UI_CONFIG_FILE: &str = "config.json";
const SKILLS_PLUGIN_REL: &str = "local-agent-mode-sessions/skills-plugin";
const SKILLS_MANIFEST_FILE: &str = "manifest.json";
const SKILLS_SUBDIR: &str = "skills";

/// Read a JSON config file. Missing file → empty object. Unparseable → error.
fn read_json_file_or_empty(path: &Path) -> Result<serde_json::Value, String> {
    match fs::read_to_string(path) {
        Ok(raw) if raw.trim().is_empty() => Ok(serde_json::json!({})),
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| format!("Parse {}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(format!("Read {}: {e}", path.display())),
    }
}

fn read_desktop_config(data_dir: &Path) -> Result<serde_json::Value, String> {
    read_json_file_or_empty(&data_dir.join(DESKTOP_CONFIG_FILE))
}

fn read_ui_config(data_dir: &Path) -> Result<serde_json::Value, String> {
    read_json_file_or_empty(&data_dir.join(UI_CONFIG_FILE))
}

/// Pretty-print `value` to `<path>.tmp` then rename over `path`. The rename
/// is atomic on the same filesystem, so readers never see a torn write.
fn write_json_atomically(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create {}: {e}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("Invalid path: {}", path.display()))?;
    let tmp = path.with_file_name(format!(".{file_name}.tmp"));
    let pretty = serde_json::to_string_pretty(value)
        .map_err(|e| format!("Serialize JSON: {e}"))?;
    fs::write(&tmp, pretty).map_err(|e| format!("Write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("Rename {} -> {}: {e}", tmp.display(), path.display()))
}

fn now_unix_millis() -> i64 {
    Utc::now().timestamp_millis()
}

// ----- MCP servers (Model B) -----

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairMcpServerShare {
    pub name: String,
    pub source_present: bool,
    pub target_present: bool,
    /// Short human-readable summary of source's value (command + first args, or url).
    pub source_summary: Option<String>,
    pub target_summary: Option<String>,
    /// True iff source and target define this server and the values are deep-equal.
    pub copied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PairMcpServerChange {
    pub name: String,
    /// New desired state: true = copy from source, false = remove from target.
    pub copied: bool,
}

fn mcp_servers_obj(config: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    config.get("mcpServers").and_then(|v| v.as_object())
}

fn mcp_server_summary(value: &serde_json::Value) -> Option<String> {
    if let Some(cmd) = value.get("command").and_then(|c| c.as_str()) {
        let argstr = value
            .get("args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .take(2)
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|s| !s.is_empty());
        Some(match argstr {
            Some(s) => format!("{cmd} {s}"),
            None => cmd.to_string(),
        })
    } else {
        value.get("url").and_then(|u| u.as_str()).map(|s| s.to_string())
    }
}

pub fn list_pair_mcp_servers(
    source_dir: &Path,
    target_dir: &Path,
) -> Result<Vec<PairMcpServerShare>, String> {
    let source_cfg = read_desktop_config(source_dir)?;
    let target_cfg = read_desktop_config(target_dir)?;
    let empty = serde_json::Map::new();
    let source_map = mcp_servers_obj(&source_cfg).unwrap_or(&empty);
    let target_map = mcp_servers_obj(&target_cfg).unwrap_or(&empty);

    let mut names: BTreeMap<String, ()> = BTreeMap::new();
    for k in source_map.keys() {
        names.insert(k.clone(), ());
    }
    for k in target_map.keys() {
        names.insert(k.clone(), ());
    }

    Ok(names
        .into_keys()
        .map(|name| {
            let src = source_map.get(&name);
            let tgt = target_map.get(&name);
            let copied = matches!((src, tgt), (Some(a), Some(b)) if a == b);
            PairMcpServerShare {
                source_summary: src.and_then(mcp_server_summary),
                target_summary: tgt.and_then(mcp_server_summary),
                source_present: src.is_some(),
                target_present: tgt.is_some(),
                copied,
                name,
            }
        })
        .collect())
}

fn set_pair_mcp_server_copied(
    source_dir: &Path,
    target_dir: &Path,
    name: &str,
    copied: bool,
) -> Result<bool, String> {
    let source_cfg = read_desktop_config(source_dir)?;
    let mut target_cfg = read_desktop_config(target_dir)?;
    let source_value = mcp_servers_obj(&source_cfg)
        .and_then(|m| m.get(name))
        .cloned();
    let target_value = mcp_servers_obj(&target_cfg)
        .and_then(|m| m.get(name))
        .cloned();
    let currently_copied = matches!((&source_value, &target_value), (Some(a), Some(b)) if a == b);
    if currently_copied == copied {
        return Ok(false);
    }

    let root = target_cfg
        .as_object_mut()
        .ok_or_else(|| "Target claude_desktop_config.json is not a JSON object".to_string())?;
    let mcp_entry = root
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let mcp_obj = mcp_entry
        .as_object_mut()
        .ok_or_else(|| "mcpServers must be an object".to_string())?;

    if copied {
        let val = source_value
            .ok_or_else(|| format!("Source has no mcpServers[\"{name}\"] to copy"))?;
        mcp_obj.insert(name.to_string(), val);
    } else {
        mcp_obj.remove(name);
    }

    write_json_atomically(&target_dir.join(DESKTOP_CONFIG_FILE), &target_cfg)?;
    Ok(true)
}

pub fn list_pair_mcp_sharing(
    source_data_dir: String,
    target_data_dir: String,
) -> Result<Vec<PairMcpServerShare>, String> {
    list_pair_mcp_servers(Path::new(&source_data_dir), Path::new(&target_data_dir))
}

pub fn apply_pair_mcp_sharing(
    source_data_dir: String,
    target_data_dir: String,
    changes: Vec<PairMcpServerChange>,
) -> Result<CopySummary, String> {
    let source = Path::new(&source_data_dir);
    let target = Path::new(&target_data_dir);
    let mut copied = 0;
    let mut skipped = 0;
    for change in changes {
        if set_pair_mcp_server_copied(source, target, &change.name, change.copied)? {
            copied += 1;
        } else {
            skipped += 1;
        }
    }
    Ok(CopySummary { copied, skipped })
}

// ----- Cowork Skills (Model A — symlink + manifest patch) -----

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairCoworkSkillShare {
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub source_present: bool,
    pub target_present: bool,
    pub source_enabled: bool,
    pub target_enabled: bool,
    /// True iff target/skills/<id> is a live symlink at source/skills/<id>
    /// AND target manifest entry matches source's.
    pub shared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PairCoworkSkillChange {
    pub skill_id: String,
    pub shared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairCoworkSkillsResult {
    pub rows: Vec<PairCoworkSkillShare>,
    /// True iff the profile has never opened the Cowork panel — sharing
    /// requires both sides to have a `<dev>/<acct>/` combo dir on disk.
    pub source_needs_bootstrap: bool,
    pub target_needs_bootstrap: bool,
}

fn skills_plugin_root(data_dir: &Path) -> PathBuf {
    let mut p = data_dir.to_path_buf();
    for segment in SKILLS_PLUGIN_REL.split('/') {
        p.push(segment);
    }
    p
}

/// Resolve the most-recently-modified `<deviceId>/<accountId>/` combo under
/// skills-plugin/. Claude Desktop writes into one combo at a time
/// (current login), and on first launch creates exactly one — so picking
/// the freshest is correct in practice.
fn find_skills_combo_dir(data_dir: &Path) -> Result<Option<PathBuf>, String> {
    let root = skills_plugin_root(data_dir);
    let outer = match fs::read_dir(&root) {
        Ok(d) => d,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Read {}: {e}", root.display())),
    };
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for outer_entry in outer {
        let outer_entry = outer_entry.map_err(|e| format!("Read skills-plugin entry: {e}"))?;
        let outer_path = outer_entry.path();
        if !outer_path.is_dir() {
            continue;
        }
        let inner = match fs::read_dir(&outer_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        for inner_entry in inner {
            let inner_entry =
                inner_entry.map_err(|e| format!("Read skills-plugin inner: {e}"))?;
            let combo = inner_entry.path();
            if !combo.is_dir() {
                continue;
            }
            let mtime = fs::metadata(&combo)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            match &best {
                None => best = Some((mtime, combo)),
                Some((bm, _)) if mtime > *bm => best = Some((mtime, combo)),
                _ => {}
            }
        }
    }
    Ok(best.map(|(_, p)| p))
}

fn read_skills_manifest(combo_dir: &Path) -> Result<serde_json::Value, String> {
    let path = combo_dir.join(SKILLS_MANIFEST_FILE);
    match fs::read_to_string(&path) {
        Ok(raw) if raw.trim().is_empty() => Ok(serde_json::json!({ "skills": [] })),
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| format!("Parse {}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(serde_json::json!({ "skills": [] })),
        Err(e) => Err(format!("Read {}: {e}", path.display())),
    }
}

fn manifest_skill_entries(manifest: &serde_json::Value) -> Vec<&serde_json::Value> {
    manifest
        .get("skills")
        .and_then(|s| s.as_array())
        .map(|arr| arr.iter().collect())
        .unwrap_or_default()
}

fn entry_skill_id(entry: &serde_json::Value) -> Option<&str> {
    entry.get("skillId").and_then(|v| v.as_str())
}

fn entry_enabled(entry: &serde_json::Value) -> bool {
    entry.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true)
}

pub fn list_pair_cowork_skills(
    source_dir: &Path,
    target_dir: &Path,
) -> Result<PairCoworkSkillsResult, String> {
    let source_combo = find_skills_combo_dir(source_dir)?;
    let target_combo = find_skills_combo_dir(target_dir)?;

    let source_manifest = match &source_combo {
        Some(p) => read_skills_manifest(p)?,
        None => serde_json::json!({ "skills": [] }),
    };
    let target_manifest = match &target_combo {
        Some(p) => read_skills_manifest(p)?,
        None => serde_json::json!({ "skills": [] }),
    };

    let mut by_id: BTreeMap<String, (Option<serde_json::Value>, Option<serde_json::Value>)> =
        BTreeMap::new();
    for entry in manifest_skill_entries(&source_manifest) {
        if let Some(id) = entry_skill_id(entry) {
            by_id.entry(id.to_string()).or_default().0 = Some(entry.clone());
        }
    }
    for entry in manifest_skill_entries(&target_manifest) {
        if let Some(id) = entry_skill_id(entry) {
            by_id.entry(id.to_string()).or_default().1 = Some(entry.clone());
        }
    }

    let mut rows = Vec::new();
    for (id, (src_entry, tgt_entry)) in by_id.into_iter() {
        let display_source = src_entry.as_ref().or(tgt_entry.as_ref());
        let name = display_source
            .and_then(|e| e.get("name").and_then(|v| v.as_str()))
            .unwrap_or(&id)
            .to_string();
        let description = display_source
            .and_then(|e| e.get("description").and_then(|v| v.as_str()))
            .map(|s| s.to_string());
        let source_enabled = src_entry.as_ref().map(entry_enabled).unwrap_or(false);
        let target_enabled = tgt_entry.as_ref().map(entry_enabled).unwrap_or(false);

        // "Shared" requires both manifest entries to match AND the on-disk
        // folder to be a symlink. Either alone is just "Independent".
        let mut shared = false;
        if let (Some(src), Some(tgt), Some(src_combo), Some(tgt_combo)) = (
            src_entry.as_ref(),
            tgt_entry.as_ref(),
            source_combo.as_ref(),
            target_combo.as_ref(),
        ) {
            let src_folder = src_combo.join(SKILLS_SUBDIR).join(&id);
            let tgt_folder = tgt_combo.join(SKILLS_SUBDIR).join(&id);
            if path_points_to(&tgt_folder, &src_folder) && src == tgt {
                shared = true;
            }
        }

        rows.push(PairCoworkSkillShare {
            source_present: src_entry.is_some(),
            target_present: tgt_entry.is_some(),
            source_enabled,
            target_enabled,
            shared,
            name,
            description,
            skill_id: id,
        });
    }

    Ok(PairCoworkSkillsResult {
        rows,
        source_needs_bootstrap: source_combo.is_none(),
        target_needs_bootstrap: target_combo.is_none(),
    })
}

fn set_pair_cowork_skill_shared(
    source_dir: &Path,
    target_dir: &Path,
    skill_id: &str,
    shared: bool,
) -> Result<bool, String> {
    let source_combo = find_skills_combo_dir(source_dir)?.ok_or_else(|| {
        "Source profile has no Cowork skills folder yet — open the Cowork panel there once."
            .to_string()
    })?;
    let target_combo = find_skills_combo_dir(target_dir)?.ok_or_else(|| {
        "Target profile has no Cowork skills folder yet — open the Cowork panel there once."
            .to_string()
    })?;

    let source_manifest = read_skills_manifest(&source_combo)?;
    let mut target_manifest = read_skills_manifest(&target_combo)?;

    let src_folder = source_combo.join(SKILLS_SUBDIR).join(skill_id);
    let tgt_folder = target_combo.join(SKILLS_SUBDIR).join(skill_id);

    let source_entry = manifest_skill_entries(&source_manifest)
        .into_iter()
        .find(|e| entry_skill_id(e) == Some(skill_id))
        .cloned();
    let target_entry = manifest_skill_entries(&target_manifest)
        .into_iter()
        .find(|e| entry_skill_id(e) == Some(skill_id))
        .cloned();

    let currently_shared = path_points_to(&tgt_folder, &src_folder)
        && source_entry.is_some()
        && source_entry == target_entry;
    if currently_shared == shared {
        return Ok(false);
    }

    if shared {
        let entry = source_entry
            .ok_or_else(|| format!("Source manifest has no entry for \"{skill_id}\""))?;
        if !src_folder.exists() && fs::symlink_metadata(&src_folder).is_err() {
            return Err(format!("Source skill folder missing: {}", src_folder.display()));
        }
        fs::create_dir_all(target_combo.join(SKILLS_SUBDIR))
            .map_err(|e| format!("Create target skills dir: {e}"))?;
        if tgt_folder.exists() || fs::symlink_metadata(&tgt_folder).is_ok() {
            backup_existing_path(&tgt_folder, target_dir, skill_id)?;
        }
        symlink_path(&src_folder, &tgt_folder)?;

        let arr = target_manifest
            .get_mut("skills")
            .and_then(|s| s.as_array_mut())
            .ok_or_else(|| "Target manifest missing skills array".to_string())?;
        if let Some(pos) = arr.iter().position(|e| entry_skill_id(e) == Some(skill_id)) {
            arr[pos] = entry;
        } else {
            arr.push(entry);
        }
    } else {
        if path_points_to(&tgt_folder, &src_folder) {
            remove_path(&tgt_folder)?;
        }
        let arr = target_manifest
            .get_mut("skills")
            .and_then(|s| s.as_array_mut())
            .ok_or_else(|| "Target manifest missing skills array".to_string())?;
        arr.retain(|e| entry_skill_id(e) != Some(skill_id));
    }

    // Bump lastUpdated so Desktop reloads the manifest on next read.
    target_manifest
        .as_object_mut()
        .ok_or_else(|| "Target manifest is not a JSON object".to_string())?
        .insert("lastUpdated".to_string(), serde_json::json!(now_unix_millis()));
    write_json_atomically(&target_combo.join(SKILLS_MANIFEST_FILE), &target_manifest)?;
    Ok(true)
}

pub fn list_pair_cowork_skills_sharing(
    source_data_dir: String,
    target_data_dir: String,
) -> Result<PairCoworkSkillsResult, String> {
    list_pair_cowork_skills(Path::new(&source_data_dir), Path::new(&target_data_dir))
}

pub fn apply_pair_cowork_skills_sharing(
    source_data_dir: String,
    target_data_dir: String,
    changes: Vec<PairCoworkSkillChange>,
) -> Result<CopySummary, String> {
    let source = Path::new(&source_data_dir);
    let target = Path::new(&target_data_dir);
    let mut copied = 0;
    let mut skipped = 0;
    for change in changes {
        if set_pair_cowork_skill_shared(source, target, &change.skill_id, change.shared)? {
            copied += 1;
        } else {
            skipped += 1;
        }
    }
    Ok(CopySummary { copied, skipped })
}

// ----- Preferences (Model B, with key allowlist) -----

const SAFE_UI_KEYS: &[&str] = &["darkMode", "scale", "multiTitleBar"];
const SAFE_DESKTOP_PREF_KEYS: &[&str] = &[
    "menuBarEnabled",
    "quickEntryShortcut",
    "chicagoEnabled",
    "sidebarMode",
    "remoteToolsDeviceName",
    "coworkScheduledTasksEnabled",
    "ccdScheduledTasksEnabled",
    "coworkWebSearchEnabled",
    "launchPreviewPersistSession",
];

// `serde_json::Value` only implements `PartialEq`, not `Eq` (because of f64
// NaN), so this struct deliberately doesn't derive Eq.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PairPreferenceShare {
    pub key: String,
    /// "ui" → top-level key in config.json.
    /// "desktop_pref" → key under "preferences" in claude_desktop_config.json.
    pub scope: String,
    pub label: String,
    pub source_present: bool,
    pub target_present: bool,
    pub source_value: Option<serde_json::Value>,
    pub target_value: Option<serde_json::Value>,
    pub copied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PairPreferenceChange {
    pub key: String,
    pub scope: String,
    pub copied: bool,
}

fn pref_label(key: &str) -> String {
    // camelCase → "Sentence case with spaces". Cheap humanization.
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if i == 0 {
            out.push(c.to_ascii_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn lookup_pref(
    scope: &str,
    key: &str,
    ui: &serde_json::Value,
    desktop: &serde_json::Value,
) -> Option<serde_json::Value> {
    match scope {
        "ui" => ui.get(key).cloned(),
        "desktop_pref" => desktop.get("preferences").and_then(|p| p.get(key)).cloned(),
        _ => None,
    }
}

pub fn list_pair_preferences(
    source_dir: &Path,
    target_dir: &Path,
) -> Result<Vec<PairPreferenceShare>, String> {
    let source_ui = read_ui_config(source_dir)?;
    let target_ui = read_ui_config(target_dir)?;
    let source_desktop = read_desktop_config(source_dir)?;
    let target_desktop = read_desktop_config(target_dir)?;

    let entries: Vec<(&str, &str)> = SAFE_UI_KEYS
        .iter()
        .map(|k| ("ui", *k))
        .chain(SAFE_DESKTOP_PREF_KEYS.iter().map(|k| ("desktop_pref", *k)))
        .collect();

    Ok(entries
        .into_iter()
        .map(|(scope, key)| {
            let src = lookup_pref(scope, key, &source_ui, &source_desktop);
            let tgt = lookup_pref(scope, key, &target_ui, &target_desktop);
            let copied = matches!((&src, &tgt), (Some(a), Some(b)) if a == b);
            PairPreferenceShare {
                source_present: src.is_some(),
                target_present: tgt.is_some(),
                source_value: src,
                target_value: tgt,
                copied,
                label: pref_label(key),
                scope: scope.to_string(),
                key: key.to_string(),
            }
        })
        .collect())
}

fn set_pair_preference_copied(
    source_dir: &Path,
    target_dir: &Path,
    key: &str,
    scope: &str,
    copied: bool,
) -> Result<bool, String> {
    let allowed = match scope {
        "ui" => SAFE_UI_KEYS.contains(&key),
        "desktop_pref" => SAFE_DESKTOP_PREF_KEYS.contains(&key),
        _ => false,
    };
    if !allowed {
        return Err(format!(
            "Preference {scope}:{key} is not in the safe allowlist"
        ));
    }

    match scope {
        "ui" => {
            let source_ui = read_ui_config(source_dir)?;
            let mut target_ui = read_ui_config(target_dir)?;
            let src_val = source_ui.get(key).cloned();
            let tgt_val = target_ui.get(key).cloned();
            let currently = matches!((&src_val, &tgt_val), (Some(a), Some(b)) if a == b);
            if currently == copied {
                return Ok(false);
            }
            let root = target_ui
                .as_object_mut()
                .ok_or_else(|| "config.json is not a JSON object".to_string())?;
            if copied {
                let v = src_val
                    .ok_or_else(|| format!("Source has no UI pref \"{key}\""))?;
                root.insert(key.to_string(), v);
            } else {
                root.remove(key);
            }
            write_json_atomically(&target_dir.join(UI_CONFIG_FILE), &target_ui)?;
            Ok(true)
        }
        "desktop_pref" => {
            let source_cfg = read_desktop_config(source_dir)?;
            let mut target_cfg = read_desktop_config(target_dir)?;
            let src_val = source_cfg.get("preferences").and_then(|p| p.get(key)).cloned();
            let tgt_val = target_cfg.get("preferences").and_then(|p| p.get(key)).cloned();
            let currently = matches!((&src_val, &tgt_val), (Some(a), Some(b)) if a == b);
            if currently == copied {
                return Ok(false);
            }
            let root = target_cfg
                .as_object_mut()
                .ok_or_else(|| "claude_desktop_config.json is not a JSON object".to_string())?;
            let prefs_entry = root
                .entry("preferences".to_string())
                .or_insert_with(|| serde_json::json!({}));
            let prefs_obj = prefs_entry
                .as_object_mut()
                .ok_or_else(|| "preferences must be an object".to_string())?;
            if copied {
                let v = src_val.ok_or_else(|| format!("Source has no pref \"{key}\""))?;
                prefs_obj.insert(key.to_string(), v);
            } else {
                prefs_obj.remove(key);
            }
            write_json_atomically(&target_dir.join(DESKTOP_CONFIG_FILE), &target_cfg)?;
            Ok(true)
        }
        _ => Err(format!("Unknown preference scope: {scope}")),
    }
}

pub fn list_pair_preference_sharing(
    source_data_dir: String,
    target_data_dir: String,
) -> Result<Vec<PairPreferenceShare>, String> {
    list_pair_preferences(Path::new(&source_data_dir), Path::new(&target_data_dir))
}

pub fn apply_pair_preference_sharing(
    source_data_dir: String,
    target_data_dir: String,
    changes: Vec<PairPreferenceChange>,
) -> Result<CopySummary, String> {
    let source = Path::new(&source_data_dir);
    let target = Path::new(&target_data_dir);
    let mut copied = 0;
    let mut skipped = 0;
    for change in changes {
        if set_pair_preference_copied(source, target, &change.key, &change.scope, change.copied)? {
            copied += 1;
        } else {
            skipped += 1;
        }
    }
    Ok(CopySummary { copied, skipped })
}

// ---------------------------------------------------------------------------
// Library views — matrix across all profiles
// ---------------------------------------------------------------------------
// The pair-wise API ships per-kind (extensions, mcp, skills, prefs, code-h).
// The "Content Library" / matrix UX needs the SAME data but reshaped: one
// row per item, one cell per (item, profile) intersection, state computed
// globally across the row so the UI can render shared/copied/diverged at a
// glance.
//
// Design ref: docs/plans/2026-05-27-content-library-grid.md

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryCell {
    pub install_id: String,
    pub install_name: String,
    pub data_dir: String,
    /// "default" | "profile"
    pub kind: String,
    /// One of: "absent" | "independent" | "copied" | "diverged" | "shared".
    /// Computed across the row by `compute_row_states`.
    pub state: String,
    pub present: bool,
    /// Short one-line preview used in tooltips and the DetailSheet.
    pub detail: Option<String>,
    /// 16-hex-char digest of the value, for diverged detection in copy-mode.
    pub digest: Option<String>,
    /// 16-hex-char digest of the symlink's resolved target, for shared-group
    /// detection in symlink-mode. None when the cell is not a symlink.
    pub link_target_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryRow {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub cells: Vec<LibraryCell>,
    /// When false, cell clicks shouldn't stage a pending toggle — the row
    /// is browse-only. We use this for per-cwd Code/Cowork rows where the
    /// sharing unit is actually the parent workspace, not the individual
    /// project. Defaults to true.
    #[serde(default = "default_true")]
    pub interactive: bool,
    /// Section bucket. Rows with the same `group` value get rendered under
    /// one bold uppercase section header in the matrix, in the style of
    /// the ProfileDetail panel's sections. None = no grouping.
    #[serde(default)]
    pub group: Option<String>,
}

#[allow(dead_code)]
fn default_true() -> bool {
    true
}

/// "user" → "User", "third-party" → "Third-party". Cheap helper used to
/// title-case the creatorType in skill-group labels.
fn other_titlecase(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LibraryCellChange {
    /// The row id (e.g. extension id, mcp server name, "ui:darkMode", skill id).
    pub row_id: String,
    /// The profile we're flipping on/off.
    pub target_install_id: String,
    /// New desired presence.
    pub wants: bool,
    /// Optional explicit source for "wants=true". When None, the apply
    /// function picks the first present sibling cell as the source.
    pub source_install_id: Option<String>,
}

/// Stable, fast (non-cryptographic) hash of a JSON value for diverged-detection.
fn value_digest(value: &serde_json::Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let s = serde_json::to_string(value).unwrap_or_default();
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// If `path` is a symlink, return a digest of its canonical resolved target;
/// otherwise None. Two cells in the same link group share the same digest.
fn symlink_target_digest(path: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_symlink() {
        return None;
    }
    let raw = fs::read_link(path).ok()?;
    let abs = if raw.is_absolute() {
        raw
    } else {
        path.parent().unwrap_or(Path::new("/")).join(raw)
    };
    let canonical = abs.canonicalize().ok()?;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut h);
    Some(format!("{:016x}", h.finish()))
}

/// Compact human preview of any JSON value — used in cell tooltips.
fn compact_value_preview(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Bool(b) => if *b { "true".into() } else { "false".into() },
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => char_truncate(s, 60),
        _ => char_truncate(&serde_json::to_string(v).unwrap_or_default(), 60),
    }
}

/// Truncate to at most `n` characters (char-safe — byte slicing panics mid-UTF-8).
fn char_truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Walk a row's cells and assign the right state to each, given whether
/// the underlying content type supports live symlink sharing.
///
/// Rules:
///   - absent: !present
///   - shared (symlink kinds only): ≥2 present cells share the same
///     link_target_digest
///   - copied (copy kinds only): another present cell has the same digest
///     AND no present cell has a different digest
///   - diverged (copy kinds only): another present cell has a different digest
///   - independent: present, but nothing else aligns
/// Copy-mode state (digest-based copied/diverged/independent). Symlink kinds use
/// `symlink_share_states` (path-based, bidirectional) instead.
fn compute_row_states(row: &mut LibraryRow) {
    let mut digest_counts: HashMap<String, usize> = HashMap::new();
    let mut present_total = 0_usize;

    for cell in &row.cells {
        if !cell.present {
            continue;
        }
        present_total += 1;
        if let Some(d) = &cell.digest {
            *digest_counts.entry(d.clone()).or_insert(0) += 1;
        }
    }

    for cell in &mut row.cells {
        if !cell.present {
            cell.state = "absent".into();
            continue;
        }
        // Copy semantics.
        if present_total <= 1 {
            cell.state = "independent".into();
            continue;
        }
        let my_digest = match &cell.digest {
            Some(d) => d,
            None => {
                cell.state = "independent".into();
                continue;
            }
        };
        let mine_total = digest_counts.get(my_digest).copied().unwrap_or(1);
        let others_same = mine_total.saturating_sub(1);
        let others_total = present_total - 1;
        let others_different = others_total.saturating_sub(others_same);
        cell.state = if others_different > 0 {
            "diverged".into()
        } else if others_same > 0 {
            "copied".into()
        } else {
            "independent".into()
        };
    }
}

// ----- Extensions library (matrix shape) -----

pub fn list_extensions_library_grid() -> Result<Vec<LibraryRow>, String> {
    let installs = list_desktop_installs()?;
    let mut ids: BTreeMap<String, ()> = BTreeMap::new();
    let mut per_install: Vec<(DesktopInstall, Vec<ExtensionEntry>)> = Vec::new();
    for install in installs {
        let exts = list_extensions_in_dir(Path::new(&install.data_dir)).unwrap_or_default();
        for e in &exts {
            ids.insert(e.id.clone(), ());
        }
        per_install.push((install, exts));
    }

    let rows: Vec<LibraryRow> = ids
        .into_keys()
        .map(|id| {
            // Per-cell on-disk path (the extension dir) drives the share state.
            let paths: Vec<PathBuf> = per_install
                .iter()
                .map(|(install, _)| Path::new(&install.data_dir).join(EXT_DIR_NAME).join(&id))
                .collect();
            let present: Vec<bool> = per_install
                .iter()
                .map(|(_, exts)| exts.iter().any(|e| e.id == id))
                .collect();
            let states = symlink_share_states(&paths, &present);
            let cells = per_install
                .iter()
                .enumerate()
                .map(|(i, (install, exts))| {
                    let entry = exts.iter().find(|e| e.id == id);
                    LibraryCell {
                        install_id: install.id.clone(),
                        install_name: install.name.clone(),
                        data_dir: install.data_dir.clone(),
                        kind: install.kind.clone(),
                        state: states[i].to_string(),
                        present: present[i],
                        detail: entry.map(|e| {
                            if e.has_settings {
                                "files+settings".into()
                            } else {
                                "files".into()
                            }
                        }),
                        digest: None,
                        link_target_digest: symlink_target_digest(&paths[i]),
                    }
                })
                .collect();
            LibraryRow {
                id: id.clone(),
                label: id,
                description: None,
                cells,
                interactive: true,
                group: None,
            }
        })
        .collect();

    Ok(rows)
}

// ----- MCP servers library -----

pub fn list_mcp_library() -> Result<Vec<LibraryRow>, String> {
    let installs = list_desktop_installs()?;
    let configs: Vec<(DesktopInstall, serde_json::Value)> = installs
        .into_iter()
        .map(|i| {
            let cfg = read_desktop_config(Path::new(&i.data_dir))
                .unwrap_or(serde_json::json!({}));
            (i, cfg)
        })
        .collect();

    let mut names: BTreeMap<String, ()> = BTreeMap::new();
    for (_, cfg) in &configs {
        if let Some(servers) = mcp_servers_obj(cfg) {
            for k in servers.keys() {
                names.insert(k.clone(), ());
            }
        }
    }

    let mut rows: Vec<LibraryRow> = names
        .into_keys()
        .map(|name| {
            let cells = configs
                .iter()
                .map(|(install, cfg)| {
                    let val = mcp_servers_obj(cfg).and_then(|s| s.get(&name));
                    LibraryCell {
                        install_id: install.id.clone(),
                        install_name: install.name.clone(),
                        data_dir: install.data_dir.clone(),
                        kind: install.kind.clone(),
                        state: String::new(),
                        present: val.is_some(),
                        detail: val.and_then(mcp_server_summary),
                        digest: val.map(value_digest),
                        link_target_digest: None,
                    }
                })
                .collect();
            LibraryRow {
                id: name.clone(),
                label: name,
                description: None,
                cells,
                interactive: true,
                group: None,
            }
        })
        .collect();

    for row in &mut rows {
        compute_row_states(row);
    }
    Ok(rows)
}

// ----- Cowork Skills library -----

pub fn list_cowork_skills_library() -> Result<Vec<LibraryRow>, String> {
    let installs = list_desktop_installs()?;
    // (install, combo_dir_if_any, manifest_value)
    let per_install: Vec<(DesktopInstall, Option<PathBuf>, serde_json::Value)> = installs
        .into_iter()
        .map(|install| {
            let data_dir = PathBuf::from(&install.data_dir);
            let combo = find_skills_combo_dir(&data_dir).unwrap_or(None);
            let manifest = match &combo {
                Some(p) => read_skills_manifest(p)
                    .unwrap_or(serde_json::json!({ "skills": [] })),
                None => serde_json::json!({ "skills": [] }),
            };
            (install, combo, manifest)
        })
        .collect();

    // Union of skill_ids, plus best-effort name/description/creatorType
    // from any manifest that has the entry. creatorType drives section
    // grouping in the UI ("Anthropic skills" vs "User skills").
    let mut ids: BTreeMap<String, (Option<String>, Option<String>, Option<String>)> = BTreeMap::new();
    for (_, _, manifest) in &per_install {
        for entry in manifest_skill_entries(manifest) {
            if let Some(id) = entry_skill_id(entry) {
                let name = entry.get("name").and_then(|v| v.as_str()).map(String::from);
                let desc = entry
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let creator = entry
                    .get("creatorType")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                ids.entry(id.into()).or_insert((name, desc, creator));
            }
        }
    }

    let rows: Vec<LibraryRow> = ids
        .into_iter()
        .map(|(id, (name, desc, creator))| {
            // Per-cell skill path (combo/skills/<id>) drives the share state.
            let paths: Vec<PathBuf> = per_install
                .iter()
                .enumerate()
                .map(|(i, (install, combo, _))| match combo {
                    Some(c) => c.join(SKILLS_SUBDIR).join(&id),
                    None => PathBuf::from(&install.data_dir).join(format!("__no_combo_{i}")),
                })
                .collect();
            let entries: Vec<Option<serde_json::Value>> = per_install
                .iter()
                .map(|(_, _, manifest)| {
                    manifest_skill_entries(manifest)
                        .into_iter()
                        .find(|e| entry_skill_id(e) == Some(&id))
                        .cloned()
                })
                .collect();
            let present: Vec<bool> = entries.iter().map(|e| e.is_some()).collect();
            let states = symlink_share_states(&paths, &present);
            let cells = per_install
                .iter()
                .enumerate()
                .map(|(i, (install, _, _))| {
                    let (detail, digest) = match &entries[i] {
                        Some(entry) => {
                            let enabled = entry_enabled(entry);
                            (
                                Some(if enabled { "enabled" } else { "disabled" }.to_string()),
                                Some(value_digest(entry)),
                            )
                        }
                        None => (None, None),
                    };
                    LibraryCell {
                        install_id: install.id.clone(),
                        install_name: install.name.clone(),
                        data_dir: install.data_dir.clone(),
                        kind: install.kind.clone(),
                        state: states[i].to_string(),
                        present: present[i],
                        detail,
                        digest,
                        link_target_digest: symlink_target_digest(&paths[i]),
                    }
                })
                .collect();
            let group_label = match creator.as_deref() {
                Some("anthropic") => "Anthropic skills".to_string(),
                Some(other) => format!("{} skills", other_titlecase(other)),
                None => "Other skills".to_string(),
            };
            LibraryRow {
                id: id.clone(),
                label: name.unwrap_or(id),
                description: desc,
                cells,
                interactive: true,
                group: Some(group_label),
            }
        })
        .collect();
    let mut rows = rows;
    // Group Anthropic-shipped skills first, third-party next, unknown last.
    rows.sort_by_key(|r| match r.group.as_deref() {
        Some("Anthropic skills") => 0,
        Some(g) if g.contains("User") => 2,
        Some(_) => 1,
        None => 9,
    });
    Ok(rows)
}

// ----- Preferences library -----

pub fn list_preferences_library() -> Result<Vec<LibraryRow>, String> {
    let installs = list_desktop_installs()?;
    let configs: Vec<(DesktopInstall, serde_json::Value, serde_json::Value)> = installs
        .into_iter()
        .map(|install| {
            let ui = read_ui_config(Path::new(&install.data_dir))
                .unwrap_or(serde_json::json!({}));
            let desktop = read_desktop_config(Path::new(&install.data_dir))
                .unwrap_or(serde_json::json!({}));
            (install, ui, desktop)
        })
        .collect();

    let mut rows: Vec<LibraryRow> = Vec::new();
    let scopes_and_keys: Vec<(&str, &str)> = SAFE_UI_KEYS
        .iter()
        .map(|k| ("ui", *k))
        .chain(SAFE_DESKTOP_PREF_KEYS.iter().map(|k| ("desktop_pref", *k)))
        .collect();

    for (scope, key) in scopes_and_keys {
        let cells = configs
            .iter()
            .map(|(install, ui, desktop)| {
                let val = match scope {
                    "ui" => ui.get(key).cloned(),
                    "desktop_pref" => desktop
                        .get("preferences")
                        .and_then(|p| p.get(key))
                        .cloned(),
                    _ => None,
                };
                LibraryCell {
                    install_id: install.id.clone(),
                    install_name: install.name.clone(),
                    data_dir: install.data_dir.clone(),
                    kind: install.kind.clone(),
                    state: String::new(),
                    present: val.is_some(),
                    detail: val.as_ref().map(compact_value_preview),
                    digest: val.as_ref().map(value_digest),
                    link_target_digest: None,
                }
            })
            .collect();
        rows.push(LibraryRow {
            id: format!("{scope}:{key}"),
            label: pref_label(key),
            description: Some(
                if scope == "ui" {
                    "config.json"
                } else {
                    "claude_desktop_config.json"
                }
                .into(),
            ),
            cells,
            interactive: true,
            group: Some(
                if scope == "ui" {
                    "UI settings"
                } else {
                    "Cowork preferences"
                }
                .into(),
            ),
        });
    }

    for row in &mut rows {
        compute_row_states(row);
    }
    Ok(rows)
}

// ----- Unified library apply -----

/// Dispatch a single cell flip to the right pair-wise apply helper.
/// Auto-picks a source: explicit `source_install_id` if provided, else the
/// first present cell in the same row that isn't the target.
// ===========================================================================
// Cross-tool Skills library (the SKILL.md dirs) — the one content surface that
// is format-compatible across Claude and Codex. Columns are the Claude Code
// config dirs (~/.claude, ~/.claude-<name>) plus ONE global Codex column
// (~/.codex/skills) — Codex skills are global (launchers set --user-data-dir,
// not CODEX_HOME), so there's a single Codex library, not one per profile.
// Sharing is a symlink, same as Claude<->Claude skill sharing.
// ===========================================================================

const CODEX_SKILLS_GLOBAL_ID: &str = "codex:global";

fn codex_skills_dir() -> Result<PathBuf, String> {
    // Codex agent config lives in $CODEX_HOME or ~/.codex (NOT the desktop
    // Chromium data dir). Skills are a CLI/agent concept stored there.
    let base = match std::env::var("CODEX_HOME") {
        Ok(h) if !h.is_empty() => PathBuf::from(h),
        _ => home_dir()?.join(".codex"),
    };
    Ok(base.join(SKILLS_SUBDIR))
}

/// On-disk skills dir backing a Skills-kind column id.
fn skills_dir_for_column(id: &str) -> Result<Option<PathBuf>, String> {
    if id == CODEX_SKILLS_GLOBAL_ID {
        return Ok(Some(codex_skills_dir()?));
    }
    for inst in list_code_installs()? {
        if inst.id == id {
            return Ok(Some(PathBuf::from(inst.config_dir).join(SKILLS_SUBDIR)));
        }
    }
    Ok(None)
}

/// Within-Claude skills (Claude tab): ~/.claude/skills across Claude code
/// accounts only — same matrix, no global Codex column.
pub fn list_claude_skills_library() -> Result<Vec<LibraryRow>, String> {
    list_skills_library_cols(None)
}

/// Cross-tool skills (Share tab): Claude code dirs + the one global Codex column.
pub fn list_skills_library() -> Result<Vec<LibraryRow>, String> {
    list_skills_library_cols(Some(codex_skills_dir()?))
}

/// Build the skills matrix over the Claude code columns, optionally appending a
/// single global Codex column (`codex_dir`). One bidirectional share-state pass.
fn list_skills_library_cols(codex_dir: Option<PathBuf>) -> Result<Vec<LibraryRow>, String> {
    let code_installs = list_code_installs()?;

    // (id, name, skills_dir) per Claude column.
    let claude_cols: Vec<(String, String, PathBuf)> = code_installs
        .iter()
        .map(|i| {
            (
                i.id.clone(),
                i.name.clone(),
                PathBuf::from(&i.config_dir).join(SKILLS_SUBDIR),
            )
        })
        .collect();

    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut collect = |dir: &Path| {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with('.') {
                    continue;
                }
                // Skill = a directory (or a symlink to one).
                if e.path().is_dir() {
                    names.insert(n);
                }
            }
        }
    };
    for (_, _, d) in &claude_cols {
        collect(d);
    }
    if let Some(cd) = &codex_dir {
        collect(cd);
    }

    let is_present = |p: &Path| {
        p.is_dir()
            || fs::symlink_metadata(p)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
    };
    let group_label = if codex_dir.is_some() {
        "Claude + Codex skills"
    } else {
        "Claude skills"
    };
    let mut rows = Vec::new();
    for name in names {
        // One bidirectional share-state pass over the Claude code dirs (+ the
        // global Codex dir when cross-tool). Real source reads "shared" too.
        let mut paths: Vec<PathBuf> = claude_cols.iter().map(|(_, _, d)| d.join(&name)).collect();
        if let Some(cd) = &codex_dir {
            paths.push(cd.join(&name));
        }
        let present: Vec<bool> = paths.iter().map(|p| is_present(p)).collect();
        let states = symlink_share_states(&paths, &present);

        let mut cells: Vec<LibraryCell> = claude_cols
            .iter()
            .enumerate()
            .map(|(i, (id, label, dir))| LibraryCell {
                install_id: id.clone(),
                install_name: label.clone(),
                data_dir: dir.to_string_lossy().to_string(),
                kind: if id == "default" { "default".into() } else { "profile".into() },
                state: states[i].to_string(),
                present: present[i],
                detail: None,
                digest: None,
                link_target_digest: symlink_target_digest(&paths[i]),
            })
            .collect();

        // The global Codex cell (last column), only for the cross-tool matrix.
        if let Some(cd) = &codex_dir {
            let ci = claude_cols.len();
            let codex_detail = if !present[ci] || states[ci] == "shared" {
                None
            } else if fs::symlink_metadata(&paths[ci])
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                Some("links elsewhere".to_string())
            } else {
                Some("real folder in ~/.codex/skills".to_string())
            };
            cells.push(LibraryCell {
                install_id: CODEX_SKILLS_GLOBAL_ID.to_string(),
                install_name: "Codex".to_string(),
                data_dir: cd.to_string_lossy().to_string(),
                kind: "codex".to_string(),
                state: states[ci].to_string(),
                present: present[ci],
                detail: codex_detail,
                digest: None,
                link_target_digest: symlink_target_digest(&paths[ci]),
            });
        }
        rows.push(LibraryRow {
            id: name.clone(),
            label: name,
            description: None,
            cells,
            interactive: true,
            group: Some(group_label.to_string()),
        });
    }
    Ok(rows)
}

/// Toggle a skill symlink for the Skills kind. Non-destructive: never
/// overwrites or deletes a real folder / foreign symlink — it refuses with a
/// clear error instead.
fn apply_skill_share(change: &LibraryCellChange) -> Result<bool, String> {
    let name = &change.row_id;
    let target_dir = skills_dir_for_column(&change.target_install_id)?
        .ok_or_else(|| format!("Unknown skills column: {}", change.target_install_id))?;
    let target_link = target_dir.join(name);

    if change.wants {
        let source_dir = if let Some(src) = &change.source_install_id {
            skills_dir_for_column(src)?.ok_or_else(|| format!("Unknown source: {src}"))?
        } else {
            // Prefer the default ~/.claude, then any present Claude column.
            let rows = list_skills_library()?;
            let row = rows
                .into_iter()
                .find(|r| r.id == *name)
                .ok_or_else(|| format!("Skill {name} not found"))?;
            let src_id = row
                .cells
                .iter()
                .find(|c| c.install_id != change.target_install_id && c.present && c.kind == "default")
                .or_else(|| {
                    row.cells.iter().find(|c| {
                        c.install_id != change.target_install_id
                            && c.present
                            && c.install_id != CODEX_SKILLS_GLOBAL_ID
                    })
                })
                .or_else(|| {
                    row.cells
                        .iter()
                        .find(|c| c.install_id != change.target_install_id && c.present)
                })
                .ok_or_else(|| "No profile holds this skill to share from.".to_string())?
                .install_id
                .clone();
            skills_dir_for_column(&src_id)?.ok_or_else(|| "Source resolution failed".to_string())?
        };
        let source_path = source_dir.join(name);
        if !source_path.is_dir() && fs::symlink_metadata(&source_path).is_err() {
            return Err(format!("Source skill \"{name}\" doesn't exist."));
        }
        if path_points_to(&target_link, &source_path) {
            return Ok(false); // already linked
        }
        if fs::symlink_metadata(&target_link).is_ok() {
            return Err(format!(
                "\"{name}\" already exists in the target and isn't a Claudex link — remove it there first."
            ));
        }
        fs::create_dir_all(&target_dir).map_err(|e| format!("Create {}: {e}", target_dir.display()))?;
        symlink_path(&source_path, &target_link)?;
        Ok(true)
    } else {
        match fs::symlink_metadata(&target_link) {
            Ok(meta) if meta.file_type().is_symlink() => {
                remove_path(&target_link)?;
                Ok(true)
            }
            Ok(_) => Err(format!(
                "\"{name}\" in the target is a real folder, not a Claudex link — leaving it untouched."
            )),
            Err(_) => Ok(false),
        }
    }
}

// ===========================================================================
// Codex content (browse) — the Codex tab's own "Content" kinds over the ONE
// global ~/.codex (sessions, skills, MCP). Codex agent config is global
// (launchers set --user-data-dir, not CODEX_HOME), so there's no between-
// Codex-profiles matrix: every row renders in a single synthetic Codex column,
// browse-only (interactive: false). Sharing them is cross-TOOL (the Share tab).
// ===========================================================================

fn codex_home_dir() -> Result<PathBuf, String> {
    match std::env::var("CODEX_HOME") {
        Ok(h) if !h.is_empty() => Ok(PathBuf::from(h)),
        _ => Ok(home_dir()?.join(".codex")),
    }
}

fn codex_config_path() -> Result<PathBuf, String> {
    Ok(codex_home_dir()?.join("config.toml"))
}

/// One Codex profile = one matrix column. `home` is the profile's CODEX_HOME
/// (default install → ~/.codex; managed → ~/.codex-<name>). `id` matches the
/// CodexInstall id ("default" / "profile:<name>") so the frontend columns line
/// up with the cells.
struct CodexCol {
    id: String,
    label: String,
    kind: String,
    home: PathBuf,
}

/// All Codex profiles as matrix columns — the default ~/.codex always first,
/// then every managed Codex profile in the registry. Now that each profile has
/// its own CODEX_HOME, sessions/skills/MCPs are per-profile, and skills/MCPs can
/// be shared BETWEEN Codex profiles (parallel to the Claude tab).
fn codex_profile_columns() -> Result<Vec<CodexCol>, String> {
    let mut cols = vec![CodexCol {
        id: "default".to_string(),
        label: "Default".to_string(),
        kind: "default".to_string(),
        home: codex_home_dir()?,
    }];
    let registry = load_registry()?;
    for p in &registry.profiles {
        if p.codex.is_some() {
            cols.push(CodexCol {
                id: format!("profile:{}", p.name),
                label: p.name.clone(),
                kind: "profile".to_string(),
                home: codex_home_dir_for(&p.name)?,
            });
        }
    }
    Ok(cols)
}

/// Resolve a Codex column id ("default" / "profile:<name>") to its CODEX_HOME.
fn codex_home_for_column(id: &str) -> Result<Option<PathBuf>, String> {
    Ok(codex_profile_columns()?.into_iter().find(|c| c.id == id).map(|c| c.home))
}

/// Import a Codex session by id, searching EVERY Codex profile home — a session
/// now lives in whichever account created it (~/.codex or ~/.codex-<name>).
pub fn import_codex_session_to_claude_any_home(
    source: String,
) -> Result<convert::ImportResult, String> {
    let cols = codex_profile_columns()?;
    let mut last_err: Option<String> = None;
    for col in &cols {
        match convert::import_codex_session_to_claude_in(&source, &col.home) {
            Ok(r) => return Ok(r),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| "No Codex session found.".to_string()))
}

/// Result of a multi-session import (project- or space-wide).
#[derive(serde::Serialize)]
pub struct BatchImportResult {
    pub imported: usize,
    pub failed: usize,
}

/// Import EVERY Codex session in one project (cwd) into Claude Code.
pub fn import_codex_project_to_claude(
    install_id: String,
    cwd: String,
) -> Result<BatchImportResult, String> {
    let sessions = list_codex_sessions_for_project(install_id, cwd)?;
    let (mut imported, mut failed) = (0, 0);
    for s in sessions {
        match import_codex_session_to_claude_any_home(s.session_id) {
            Ok(_) => imported += 1,
            Err(_) => failed += 1,
        }
    }
    Ok(BatchImportResult { imported, failed })
}

/// Import EVERY Claude Code session in one project into Codex.
pub fn import_claude_project_to_codex(
    install_id: String,
    project_id: String,
) -> Result<BatchImportResult, String> {
    let sessions = list_claude_sessions_for_project(install_id, project_id)?;
    let (mut imported, mut failed) = (0, 0);
    for s in sessions {
        match convert::import_claude_session_to_codex(s.session_id) {
            Ok(_) => imported += 1,
            Err(_) => failed += 1,
        }
    }
    Ok(BatchImportResult { imported, failed })
}

/// Import EVERY Codex session of one profile into Claude Code (space-wide).
pub fn import_all_codex_to_claude(install_id: String) -> Result<BatchImportResult, String> {
    let home = codex_home_for_column(&install_id)?
        .ok_or_else(|| format!("Unknown Codex profile: {install_id}"))?;
    let (mut imported, mut failed) = (0, 0);
    for m in scan_codex_rollouts(&home) {
        match import_codex_session_to_claude_any_home(m.session_id) {
            Ok(_) => imported += 1,
            Err(_) => failed += 1,
        }
    }
    Ok(BatchImportResult { imported, failed })
}

/// Import EVERY Claude Code session of one account into Codex (space-wide).
pub fn import_all_claude_to_codex(install_id: String) -> Result<BatchImportResult, String> {
    let config = claude_config_for_column(&install_id)?
        .ok_or_else(|| format!("Unknown Claude account: {install_id}"))?;
    let (mut imported, mut failed) = (0, 0);
    if let Ok(rd) = fs::read_dir(config.join(CODE_PROJECTS_DIR)) {
        for e in rd.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            if let Ok(rd2) = fs::read_dir(e.path()) {
                for f in rd2.flatten() {
                    let p = f.path();
                    if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                        continue;
                    }
                    let sid = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                    if sid.is_empty() {
                        continue;
                    }
                    match convert::import_claude_session_to_codex(sid) {
                        Ok(_) => imported += 1,
                        Err(_) => failed += 1,
                    }
                }
            }
        }
    }
    Ok(BatchImportResult { imported, failed })
}

const CODEX_SESSIONS_DIR: &str = "sessions";
/// Row id of the synthetic whole-`sessions/` share row.
const CODEX_ALL_SESSIONS_ID: &str = "__all_sessions__";
/// cwd bucket for rollouts whose session_meta has no cwd.
const CODEX_NO_PROJECT: &str = "(no project)";

/// One Codex session, attributed to its project (cwd). `last_activity_ms` is the
/// rollout file mtime (cheap; no full parse).
struct CodexSessionMeta {
    session_id: String,
    cwd: String,
    model: Option<String>,
    last_activity_ms: i64,
    path: PathBuf,
}

/// Scan a CODEX_HOME's sessions/ tree (head-only read per rollout for cwd/id).
fn scan_codex_rollouts(home: &Path) -> Vec<CodexSessionMeta> {
    let mut files = Vec::new();
    convert::walk_rollouts(&home.join(CODEX_SESSIONS_DIR), &mut files);
    let mut out = Vec::new();
    for f in files {
        let (id, cwd, model) = match convert::read_rollout_meta(&f) {
            Some(m) => m,
            None => continue,
        };
        let last = fs::metadata(&f)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(system_time_to_epoch_ms)
            .unwrap_or(0);
        out.push(CodexSessionMeta {
            session_id: id,
            cwd: if cwd.is_empty() { CODEX_NO_PROJECT.to_string() } else { cwd },
            model,
            last_activity_ms: last,
            path: f,
        });
    }
    out
}

/// Codex sessions, modeled like Claude's "Code sessions": one synthetic
/// interactive "__all_sessions__" row that symlink-shares the WHOLE
/// `<CODEX_HOME>/sessions/` dir between accounts, plus one browse-only row per
/// project (cwd). Drill into a project (DetailSheet) to import a session.
pub fn list_codex_sessions_library() -> Result<Vec<LibraryRow>, String> {
    let cols = codex_profile_columns()?;
    // Per-column: scanned sessions + the sessions-dir symlink digest.
    let per_col: Vec<(Vec<CodexSessionMeta>, Option<String>)> = cols
        .iter()
        .map(|c| {
            let link_d = symlink_target_digest(&c.home.join(CODEX_SESSIONS_DIR));
            (scan_codex_rollouts(&c.home), link_d)
        })
        .collect();

    // Union of project keys, sorted by most-recent activity across columns.
    let mut keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (sessions, _) in &per_col {
        for s in sessions {
            keys.insert(s.cwd.clone());
        }
    }
    let mut keys: Vec<String> = keys.into_iter().collect();
    keys.sort_by_key(|k| {
        let latest = per_col
            .iter()
            .flat_map(|(s, _)| s.iter())
            .filter(|s| &s.cwd == k)
            .map(|s| s.last_activity_ms)
            .max()
            .unwrap_or(0);
        -latest
    });

    // The share unit is the WHOLE <home>/sessions dir, so compute its share
    // state once (bidirectional, over the per-column sessions dirs) and reuse it
    // for every row.
    let ws_paths: Vec<PathBuf> = cols
        .iter()
        .map(|c| c.home.join(CODEX_SESSIONS_DIR))
        .collect();
    let ws_present: Vec<bool> = ws_paths
        .iter()
        .map(|p| {
            p.is_dir()
                || fs::symlink_metadata(p)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
        })
        .collect();
    let ws_states = symlink_share_states(&ws_paths, &ws_present);

    let mut rows: Vec<LibraryRow> = Vec::with_capacity(keys.len() + 1);

    // Synthetic whole-sessions share row.
    let all_cells: Vec<LibraryCell> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (sessions, link_d) = &per_col[i];
            let n = sessions.len();
            let last = sessions.iter().map(|s| s.last_activity_ms).max().unwrap_or(0);
            let detail = if n > 0 {
                Some(format!(
                    "{n} session{} · {}",
                    if n == 1 { "" } else { "s" },
                    if last > 0 { humanize_ago(last) } else { "—".into() }
                ))
            } else {
                None
            };
            LibraryCell {
                install_id: c.id.clone(),
                install_name: c.label.clone(),
                data_dir: c.home.join(CODEX_SESSIONS_DIR).to_string_lossy().to_string(),
                kind: c.kind.clone(),
                state: if ws_present[i] { ws_states[i].to_string() } else { "absent".to_string() },
                present: ws_present[i],
                detail,
                digest: None,
                link_target_digest: link_d.clone(),
            }
        })
        .collect();
    let all_row = LibraryRow {
        id: CODEX_ALL_SESSIONS_ID.to_string(),
        label: "All Codex sessions".into(),
        description: Some("Toggle to symlink the whole ~/.codex/sessions dir between accounts.".into()),
        cells: all_cells,
        interactive: true,
        group: Some("Sessions".into()),
    };
    rows.push(all_row);

    // One browse row per project (cwd).
    let home = std::env::var("HOME").unwrap_or_default();
    for key in keys {
        let cells: Vec<LibraryCell> = cols
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let (sessions, link_d) = &per_col[i];
                let matching: Vec<&CodexSessionMeta> =
                    sessions.iter().filter(|s| s.cwd == key).collect();
                let n = matching.len();
                let last = matching.iter().map(|s| s.last_activity_ms).max().unwrap_or(0);
                let detail = if n > 0 {
                    Some(format!(
                        "{n} session{} · {}",
                        if n == 1 { "" } else { "s" },
                        if last > 0 { humanize_ago(last) } else { "—".into() }
                    ))
                } else {
                    None
                };
                LibraryCell {
                    install_id: c.id.clone(),
                    install_name: c.label.clone(),
                    data_dir: c.home.join(CODEX_SESSIONS_DIR).to_string_lossy().to_string(),
                    kind: c.kind.clone(),
                    // Sharing is whole-sessions-dir; a project cell shows the
                    // workspace share state when it has sessions here, else absent.
                    state: if n > 0 { ws_states[i].to_string() } else { "absent".to_string() },
                    present: n > 0,
                    detail,
                    digest: None,
                    link_target_digest: link_d.clone(),
                }
            })
            .collect();
        let label = if key == CODEX_NO_PROJECT {
            CODEX_NO_PROJECT.to_string()
        } else {
            Path::new(&key)
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or_else(|| key.clone())
        };
        let description = if key == CODEX_NO_PROJECT {
            None
        } else {
            Some(key.replace(&home, "~"))
        };
        rows.push(LibraryRow {
            id: key,
            label,
            description,
            cells,
            interactive: false,
            group: Some("Projects".into()),
        });
    }

    rows.sort_by_key(|r| match r.group.as_deref() {
        Some("Sessions") => 0,
        Some("Projects") => 1,
        _ => 9,
    });
    Ok(rows)
}

/// Drill-down: individual Claude Code CLI sessions in one project for one
/// account (~/.claude[-<name>]/projects/<project_id>/*.jsonl).
pub fn list_claude_sessions_for_project(
    install_id: String,
    project_id: String,
) -> Result<Vec<LocalSession>, String> {
    let config = claude_config_for_column(&install_id)?
        .ok_or_else(|| format!("Unknown Claude account: {install_id}"))?;
    let projects_root = config.join(CODE_PROJECTS_DIR);
    // Grouped project rows carry a root cwd (absolute path) as their id — gather
    // sessions from every dir under that root (the project + all its worktrees).
    // A non-absolute id is a legacy single dir name.
    let dirs: Vec<PathBuf> = if project_id.starts_with('/') {
        fs::read_dir(&projects_root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter(|p| {
                project_cwd_from_sessions(p)
                    .map(|c| worktree_root(&c) == project_id)
                    .unwrap_or(false)
            })
            .collect()
    } else {
        vec![projects_root.join(&project_id)]
    };
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let sid = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if sid.is_empty() {
                continue;
            }
            let last = fs::metadata(&p)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(system_time_to_epoch_ms)
                .unwrap_or(0);
            out.push(LocalSession {
                session_id: sid,
                // Title = the first real user prompt (cleanly one-lined, injected
                // context skipped) instead of the bare UUID file stem.
                title: read_claude_session_title(&p),
                cwd: Some(project_id.clone()),
                process_name: None,
                model: None,
                created_at_ms: last,
                last_activity_ms: last,
                account_name: None,
                email_address: None,
            });
        }
    }
    out.sort_by_key(|s| -s.last_activity_ms);
    Ok(out)
}

/// Drill-down: individual Codex sessions in one project (cwd) for one account.
/// Mapped into LocalSession so the DetailSheet reuses the Claude session list.
pub fn list_codex_sessions_for_project(
    install_id: String,
    cwd: String,
) -> Result<Vec<LocalSession>, String> {
    let home = codex_home_for_column(&install_id)?
        .ok_or_else(|| format!("Unknown Codex profile: {install_id}"))?;
    let mut sessions: Vec<LocalSession> = scan_codex_rollouts(&home)
        .into_iter()
        .filter(|s| s.cwd == cwd)
        .map(|s| LocalSession {
            // Only the sessions for THIS cwd reach here (filtered above), so
            // reading each one's first user message for the title is cheap.
            title: read_codex_session_title(&s.path),
            session_id: s.session_id,
            cwd: Some(s.cwd),
            process_name: None,
            model: s.model,
            created_at_ms: s.last_activity_ms,
            last_activity_ms: s.last_activity_ms,
            account_name: None,
            email_address: None,
        })
        .collect();
    sessions.sort_by_key(|s| -s.last_activity_ms);
    Ok(sessions)
}

/// Symlink the whole `<source_home>/sessions` into `<target_home>/sessions`.
/// Resolve a path to its REAL underlying directory. For an existing path this is
/// `fs::canonicalize`. For a not-yet-created leaf (a symlink slot we're about to
/// write) it canonicalizes the PARENT and rejoins the leaf, so source and target
/// can be compared on real paths even before the link exists.
fn real_dir_of(p: &Path) -> Option<PathBuf> {
    if let Ok(rp) = fs::canonicalize(p) {
        return Some(rp);
    }
    let parent = p.parent()?;
    let leaf = p.file_name()?;
    fs::canonicalize(parent).ok().map(|rp| rp.join(leaf))
}

/// Linking `target -> source` self-links or cycles iff their REAL (canonicalized)
/// paths are equal — for sibling sessions/ or projects/ dirs a cycle always
/// collapses to equality after canonicalize, so this one test is sufficient.
fn would_cycle_or_selflink(real_source: &Path, real_target: &Path) -> bool {
    real_source == real_target
}

/// Count *.jsonl files recursively under `dir` (Codex rollouts / Claude
/// sessions). DirEntry file types are not symlink-followed, so a symlinked
/// sub-dir is not descended — pass a REAL (canonicalized) root.
fn count_jsonl_under(dir: &Path) -> usize {
    let mut n = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            match e.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(e.path()),
                _ => {
                    if e.path().extension().and_then(|x| x.to_str()) == Some("jsonl") {
                        n += 1;
                    }
                }
            }
        }
    }
    n
}

/// Rollout count in a Codex account's REAL sessions dir (resolved through any
/// symlink so a freshly-pooled spoke is counted by the hub it points at).
fn codex_real_rollout_count(home: &Path) -> usize {
    match real_dir_of(&home.join(CODEX_SESSIONS_DIR)) {
        Some(d) if d.is_dir() => count_jsonl_under(&d),
        _ => 0,
    }
}

/// Session (.jsonl) count in a Claude account's REAL projects dir.
fn claude_real_session_count(config: &Path) -> usize {
    match real_dir_of(&config.join(CODE_PROJECTS_DIR)) {
        Some(d) if d.is_dir() => count_jsonl_under(&d),
        _ => 0,
    }
}

/// Pick the richest account id among `candidates`, looking it up in
/// `(id, is_default, count)` triples; ties prefer the default account.
fn pick_richest_source(
    candidates: &[String],
    counts: &[(String, bool, usize)],
) -> Option<String> {
    candidates
        .iter()
        .filter_map(|id| counts.iter().find(|(cid, _, _)| cid == id))
        .max_by(|a, b| a.2.cmp(&b.2).then(a.1.cmp(&b.1)))
        .map(|(id, _, _)| id.clone())
}

/// Collapse a batch of "All sessions" SHARE toggles into a star topology: ONE
/// canonical real source (the richest account) plus every other staged account
/// linked to it. This closes the data-loss hole where toggling share on two
/// independent accounts in one Apply made the sequential per-change auto-pick
/// choose each other and form a circular symlink (both dirs displaced to
/// backups → "all sessions gone"). Non-sessions kinds, single-toggle batches,
/// and un-share batches pass through untouched.
fn canonicalize_sessions_batch(
    kind: &str,
    changes: Vec<LibraryCellChange>,
) -> Result<Vec<LibraryCellChange>, String> {
    let (share_on, rest): (Vec<_>, Vec<_>) = changes
        .into_iter()
        .partition(|c| c.row_id == CODEX_ALL_SESSIONS_ID && c.wants);
    if share_on.len() < 2 {
        let mut v = rest;
        v.extend(share_on);
        return Ok(v);
    }
    let target_ids: Vec<String> = share_on.iter().map(|c| c.target_install_id.clone()).collect();
    // An explicit source on any staged change wins (the UI sends none today).
    let canonical_id = if let Some(explicit) = share_on.iter().find_map(|c| c.source_install_id.clone()) {
        explicit
    } else {
        let counts: Vec<(String, bool, usize)> = if kind == "codex_sessions" {
            codex_profile_columns()?
                .iter()
                .map(|c| (c.id.clone(), c.kind == "default", codex_real_rollout_count(&c.home)))
                .collect()
        } else {
            claude_session_cols()?
                .iter()
                .map(|(id, _, dir)| (id.clone(), id == "default", claude_real_session_count(dir)))
                .collect()
        };
        pick_richest_source(&target_ids, &counts)
            .ok_or_else(|| "No source account available to share from.".to_string())?
    };
    // Drop the canonical's own toggle (a source never links to itself); rewrite
    // every other staged account to link explicitly to the canonical hub.
    let mut out = rest;
    for c in share_on {
        if c.target_install_id == canonical_id {
            continue;
        }
        out.push(LibraryCellChange {
            row_id: CODEX_ALL_SESSIONS_ID.to_string(),
            target_install_id: c.target_install_id,
            wants: true,
            source_install_id: Some(canonical_id.clone()),
        });
    }
    Ok(out)
}

fn share_codex_sessions(source_home: &Path, target_home: &Path) -> Result<bool, String> {
    let source = source_home.join(CODEX_SESSIONS_DIR);
    let target = target_home.join(CODEX_SESSIONS_DIR);
    if !source.is_dir() {
        return Err("Source account has no sessions/ dir yet.".to_string());
    }
    if path_points_to(&target, &source) {
        return Ok(false); // already shared (cheap one-hop check)
    }
    // Ensure the target CODEX_HOME exists first, else symlink(2) fails with
    // ENOENT and the share silently never happens (same class of bug as
    // share_claude_sessions). Non-destructive: only the parent, never the
    // sessions/ link target.
    fs::create_dir_all(target_home)
        .map_err(|e| format!("Create {}: {e}", target_home.display()))?;
    // DEFENSE IN DEPTH: resolve both ends to their REAL dirs and refuse any
    // self/cycle link BEFORE displacing anything, and always link to the
    // canonicalized real source so a symlink chain can never lengthen.
    let real_source = real_dir_of(&source)
        .filter(|p| p.is_dir())
        .ok_or_else(|| "Source sessions/ does not resolve to a real directory.".to_string())?;
    let real_target =
        real_dir_of(&target).ok_or_else(|| "Cannot resolve target sessions/ path.".to_string())?;
    let target_is_link = fs::symlink_metadata(&target)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if target_is_link && real_target == real_source {
        return Ok(false); // chain already resolves to the source — nothing to do
    }
    if would_cycle_or_selflink(&real_source, &real_target) {
        return Err(format!(
            "Refusing to share: linking {} → {} would create a cycle or self-link.",
            target.display(),
            source.display()
        ));
    }
    // Guard passed — only now displace any real target dir, then link. Link to
    // the RAW source path (not the canonicalized one) so the path-based share
    // detection (symlink_share_states / path_points_to, which compares against
    // sibling raw paths) keeps reading "shared". Layer 1 guarantees the source
    // is a real dir, so this never lengthens a chain.
    if fs::symlink_metadata(&target).is_ok() {
        backup_existing_path(&target, target_home, CODEX_SESSIONS_DIR)?;
    }
    symlink_path(&source, &target)?;
    Ok(true)
}

/// Undo a sessions symlink: remove it and copy back the content it ACTUALLY
/// pointed at (resolved from the symlink itself, not a guessed source — with
/// 3+ accounts the "first other column" can be the wrong one). The target ends
/// up standalone with the exact sessions it was showing.
fn make_codex_sessions_independent(target_home: &Path) -> Result<bool, String> {
    let target = target_home.join(CODEX_SESSIONS_DIR);
    match fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => {
            // Resolve the real directory the symlink points to BEFORE removing it.
            let real = fs::read_link(&target).ok().map(|link| {
                if link.is_absolute() {
                    link
                } else {
                    target.parent().unwrap_or(Path::new("/")).join(link)
                }
            });
            remove_path(&target)?;
            if let Some(real) = real {
                if real.is_dir() {
                    copy_dir_recursive(&real, &target)?;
                }
            }
            Ok(true)
        }
        _ => Ok(false), // not a symlink — nothing to undo
    }
}

/// Apply the whole-sessions share toggle. Only the synthetic row is actionable;
/// per-project rows are browse-only (no per-project symlink boundary exists).
fn apply_codex_sessions_share(change: &LibraryCellChange) -> Result<bool, String> {
    if change.row_id != CODEX_ALL_SESSIONS_ID {
        return Err(
            "Per-project Codex rows are browse-only — toggle 'All Codex sessions' to share, or drill in to import a session."
                .to_string(),
        );
    }
    let cols = codex_profile_columns()?;
    let target_home = cols
        .iter()
        .find(|c| c.id == change.target_install_id)
        .map(|c| c.home.clone())
        .ok_or_else(|| format!("Unknown Codex profile: {}", change.target_install_id))?;

    if change.wants {
        let source_home = if let Some(src) = &change.source_install_id {
            cols.iter()
                .find(|c| &c.id == src)
                .map(|c| c.home.clone())
                .ok_or_else(|| format!("Unknown source: {src}"))?
        } else {
            // First other column that has a sessions dir with content.
            cols.iter()
                .find(|c| {
                    c.id != change.target_install_id
                        && c.home.join(CODEX_SESSIONS_DIR).is_dir()
                        && !scan_codex_rollouts(&c.home).is_empty()
                })
                .map(|c| c.home.clone())
                .ok_or_else(|| "No account has sessions to share from.".to_string())?
        };
        share_codex_sessions(&source_home, &target_home)
    } else {
        // Resolve the real link target from the symlink itself — no guessing.
        let target = target_home.join(CODEX_SESSIONS_DIR);
        let target_is_link = fs::symlink_metadata(&target)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if target_is_link {
            make_codex_sessions_independent(&target_home)
        } else {
            // Target is the REAL source — detach every sibling linking into it so
            // un-share works from either cell (both render "shared").
            let mut changed = false;
            for c in &cols {
                if c.home != target_home
                    && path_points_to(&c.home.join(CODEX_SESSIONS_DIR), &target)
                    && make_codex_sessions_independent(&c.home)?
                {
                    changed = true;
                }
            }
            Ok(changed)
        }
    }
}

// ===========================================================================
// Claude Code sessions (Claude tab) — the CLI store ~/.claude/projects, one
// column per Claude code account (~/.claude, ~/.claude-<name>). Same model as
// Codex sessions: a synthetic "All sessions" row that symlink-shares the whole
// projects/ dir between accounts, plus browse-only per-project rows.
// ===========================================================================

/// (id, name, config_dir) per Claude code account.
fn claude_session_cols() -> Result<Vec<(String, String, PathBuf)>, String> {
    Ok(list_code_installs()?
        .into_iter()
        .map(|i| (i.id, i.name, PathBuf::from(i.config_dir)))
        .collect())
}

/// Resolve a Claude code column id to its config dir.
fn claude_config_for_column(id: &str) -> Result<Option<PathBuf>, String> {
    Ok(claude_session_cols()?.into_iter().find(|(cid, _, _)| cid == id).map(|(_, _, d)| d))
}

pub fn list_claude_sessions_library() -> Result<Vec<LibraryRow>, String> {
    let cols = claude_session_cols()?;
    // Per-account projects + whole projects-dir present/state.
    let per_col: Vec<Vec<CodeProject>> = cols
        .iter()
        .map(|(_, _, dir)| list_code_history(dir).unwrap_or_default())
        .collect();
    let ws_paths: Vec<PathBuf> = cols.iter().map(|(_, _, d)| d.join(CODE_PROJECTS_DIR)).collect();
    let ws_present: Vec<bool> = ws_paths
        .iter()
        .map(|p| {
            p.is_dir()
                || fs::symlink_metadata(p)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
        })
        .collect();
    let ws_states = symlink_share_states(&ws_paths, &ws_present);

    // Group projects by their ROOT cwd, collapsing git worktrees
    // (<project>/.claude/worktrees/<name>) into the parent. Each worktree has its
    // own ~/.claude/projects entry; without this they explode into 20+ rows of
    // random worktree codenames instead of one project row. Empty dirs (0
    // sessions) are dropped so they don't clutter the list either.
    let mut roots: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for projects in &per_col {
        for p in projects {
            if p.session_count == 0 {
                continue;
            }
            let e = roots
                .entry(worktree_root(&p.display_path).to_string())
                .or_insert(0);
            *e = (*e).max(p.last_modified_ms);
        }
    }
    let mut roots: Vec<(String, i64)> = roots.into_iter().collect();
    roots.sort_by_key(|(_, t)| -*t);

    let mut rows: Vec<LibraryRow> = Vec::with_capacity(roots.len() + 1);

    // Synthetic whole-projects share row.
    let all_cells: Vec<LibraryCell> = cols
        .iter()
        .enumerate()
        .map(|(i, (id, label, _))| {
            let n: u32 = per_col[i].iter().map(|p| p.session_count).sum();
            let last = per_col[i].iter().map(|p| p.last_modified_ms).max().unwrap_or(0);
            let detail = if n > 0 {
                Some(format!(
                    "{n} session{} · {}",
                    if n == 1 { "" } else { "s" },
                    if last > 0 { humanize_ago(last) } else { "—".into() }
                ))
            } else {
                None
            };
            LibraryCell {
                install_id: id.clone(),
                install_name: label.clone(),
                data_dir: ws_paths[i].to_string_lossy().to_string(),
                kind: if id == "default" { "default".into() } else { "profile".into() },
                state: if ws_present[i] { ws_states[i].to_string() } else { "absent".to_string() },
                present: ws_present[i],
                detail,
                digest: None,
                link_target_digest: symlink_target_digest(&ws_paths[i]),
            }
        })
        .collect();
    rows.push(LibraryRow {
        id: CODEX_ALL_SESSIONS_ID.to_string(),
        label: "All Claude sessions".into(),
        description: Some("Toggle to symlink the whole ~/.claude/projects dir between accounts.".into()),
        cells: all_cells,
        interactive: true,
        group: Some("Sessions".into()),
    });

    // One browse row per project (root). Cells aggregate every dir in the group
    // (the project itself + all its worktrees) for that account.
    let home = std::env::var("HOME").unwrap_or_default();
    for (root, _) in roots {
        let cells: Vec<LibraryCell> = cols
            .iter()
            .enumerate()
            .map(|(i, (id, label, _))| {
                let n: u32 = per_col[i]
                    .iter()
                    .filter(|p| worktree_root(&p.display_path) == root)
                    .map(|p| p.session_count)
                    .sum();
                let last = per_col[i]
                    .iter()
                    .filter(|p| worktree_root(&p.display_path) == root)
                    .map(|p| p.last_modified_ms)
                    .max()
                    .unwrap_or(0);
                let detail = if n > 0 {
                    Some(format!(
                        "{n} session{} · {}",
                        if n == 1 { "" } else { "s" },
                        if last > 0 { humanize_ago(last) } else { "—".into() }
                    ))
                } else {
                    None
                };
                LibraryCell {
                    install_id: id.clone(),
                    install_name: label.clone(),
                    data_dir: ws_paths[i].to_string_lossy().to_string(),
                    kind: if id == "default" { "default".into() } else { "profile".into() },
                    state: if n > 0 { ws_states[i].to_string() } else { "absent".to_string() },
                    present: n > 0,
                    detail,
                    digest: None,
                    link_target_digest: symlink_target_digest(&ws_paths[i]),
                }
            })
            .collect();
        let label = Path::new(&root)
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| root.clone());
        rows.push(LibraryRow {
            id: root.clone(),
            label,
            description: Some(root.replace(&home, "~")),
            cells,
            interactive: false,
            group: Some("Projects".into()),
        });
    }

    rows.sort_by_key(|r| match r.group.as_deref() {
        Some("Sessions") => 0,
        Some("Projects") => 1,
        _ => 9,
    });
    Ok(rows)
}

fn share_claude_sessions(source_config: &Path, target_config: &Path) -> Result<bool, String> {
    let source = source_config.join(CODE_PROJECTS_DIR);
    let target = target_config.join(CODE_PROJECTS_DIR);
    if !source.is_dir() {
        return Err("Source account has no projects/ dir yet.".to_string());
    }
    if path_points_to(&target, &source) {
        return Ok(false); // already shared — idempotent
    }
    // Ensure the TARGET ACCOUNT dir (the symlink's parent, e.g. ~/.claude-judy)
    // exists first, or symlink(2) fails with ENOENT and the share silently never
    // happens — exactly why a derived-but-absent account (JUDY) read "independent"
    // after the user thought they'd shared. Every other symlink-apply path already
    // does this. create_dir_all is a no-op when present and only makes the PARENT,
    // never the projects/ link target — so a real projects dir is never clobbered.
    fs::create_dir_all(target_config)
        .map_err(|e| format!("Create {}: {e}", target_config.display()))?;
    // DEFENSE IN DEPTH (twin of share_codex_sessions): resolve to REAL dirs,
    // refuse any self/cycle link BEFORE backup, and link to the real source so
    // the symlink chain can never lengthen.
    let real_source = real_dir_of(&source)
        .filter(|p| p.is_dir())
        .ok_or_else(|| "Source projects/ does not resolve to a real directory.".to_string())?;
    let real_target =
        real_dir_of(&target).ok_or_else(|| "Cannot resolve target projects/ path.".to_string())?;
    let target_is_link = fs::symlink_metadata(&target)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if target_is_link && real_target == real_source {
        return Ok(false);
    }
    if would_cycle_or_selflink(&real_source, &real_target) {
        return Err(format!(
            "Refusing to share: linking {} → {} would create a cycle or self-link.",
            target.display(),
            source.display()
        ));
    }
    if fs::symlink_metadata(&target).is_ok() {
        backup_existing_path(&target, target_config, CODE_PROJECTS_DIR)?;
    }
    // Link to the RAW source so path-based detection keeps reading "shared"
    // (see share_codex_sessions). Layer 1 guarantees source is a real dir.
    symlink_path(&source, &target)?;
    Ok(true)
}

fn apply_claude_sessions_share(change: &LibraryCellChange) -> Result<bool, String> {
    if change.row_id != CODEX_ALL_SESSIONS_ID {
        return Err(
            "Per-project Claude rows are browse-only — toggle 'All Claude sessions' to share, or export one in Share."
                .to_string(),
        );
    }
    let cols = claude_session_cols()?;
    let target_config = cols
        .iter()
        .find(|(id, _, _)| id == &change.target_install_id)
        .map(|(_, _, d)| d.clone())
        .ok_or_else(|| format!("Unknown Claude account: {}", change.target_install_id))?;
    if change.wants {
        let source_config = if let Some(src) = &change.source_install_id {
            claude_config_for_column(src)?.ok_or_else(|| format!("Unknown source: {src}"))?
        } else {
            cols.iter()
                .find(|(id, _, d)| {
                    id != &change.target_install_id && d.join(CODE_PROJECTS_DIR).is_dir()
                })
                .map(|(_, _, d)| d.clone())
                .ok_or_else(|| "No account has sessions to share from.".to_string())?
        };
        share_claude_sessions(&source_config, &target_config)
    } else {
        let target = target_config.join(CODE_PROJECTS_DIR);
        let target_is_link = fs::symlink_metadata(&target)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if target_is_link {
            make_dir_symlink_independent(&target)
        } else {
            // Target is the REAL source dir that others link into. Both the
            // source and its linkers render "shared", so the user can't tell
            // which side holds the link — make un-sharing work from EITHER cell
            // by detaching every sibling that points at this source.
            let mut changed = false;
            for (_, _, d) in &cols {
                let other = d.join(CODE_PROJECTS_DIR);
                if other != target
                    && path_points_to(&other, &target)
                    && make_dir_symlink_independent(&other)?
                {
                    changed = true;
                }
            }
            Ok(changed)
        }
    }
}

// ===========================================================================
// Codex Preferences (Codex tab) — shareable behavior knobs from config.toml,
// copy-mode between Codex homes. Auth + active-profile + [mcp_servers] are
// excluded (auth is per-profile by design; MCP is its own kind).
// ===========================================================================

const SAFE_CODEX_PREF_KEYS: &[&str] = &[
    "model",
    "model_provider",
    "model_reasoning_effort",
    "model_reasoning_summary",
    "model_verbosity",
    "approval_policy",
    "sandbox_mode",
    "hide_agent_reasoning",
    "file_opener",
    "disable_response_storage",
];

/// Read the allowlisted top-level scalar prefs from a config.toml as JSON values.
fn read_codex_prefs_at(path: &Path) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(_) => return out,
    };
    let doc: toml_edit::ImDocument<String> = match raw.parse() {
        Ok(d) => d,
        Err(_) => return out,
    };
    for key in SAFE_CODEX_PREF_KEYS {
        if let Some(item) = doc.get(key) {
            if item.is_value() {
                out.insert(key.to_string(), toml_item_to_json(item));
            }
        }
    }
    out
}

pub fn list_codex_preferences_library() -> Result<Vec<LibraryRow>, String> {
    let cols = codex_profile_columns()?;
    let maps: Vec<BTreeMap<String, serde_json::Value>> = cols
        .iter()
        .map(|c| read_codex_prefs_at(&c.home.join("config.toml")))
        .collect();
    let mut rows = Vec::new();
    for key in SAFE_CODEX_PREF_KEYS {
        // Only show a key if at least one account sets it.
        if !maps.iter().any(|m| m.contains_key(*key)) {
            continue;
        }
        let cells: Vec<LibraryCell> = cols
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let v = maps[i].get(*key);
                LibraryCell {
                    install_id: c.id.clone(),
                    install_name: c.label.clone(),
                    data_dir: c.home.join("config.toml").to_string_lossy().to_string(),
                    kind: c.kind.clone(),
                    state: String::new(),
                    present: v.is_some(),
                    detail: v.map(compact_value_preview),
                    digest: v.map(value_digest),
                    link_target_digest: None,
                }
            })
            .collect();
        let mut row = LibraryRow {
            id: key.to_string(),
            label: key.to_string(),
            description: None,
            cells,
            interactive: true,
            group: None,
        };
        compute_row_states(&mut row);
        rows.push(row);
    }
    Ok(rows)
}

/// Copy a preference key's value from the source account's config.toml into the
/// target's (copy-mode). wants=false removes the key from the target.
fn apply_codex_preferences_share(change: &LibraryCellChange) -> Result<bool, String> {
    let key = change.row_id.as_str();
    if !SAFE_CODEX_PREF_KEYS.contains(&key) {
        return Err(format!("\"{key}\" is not a shareable Codex preference."));
    }
    let cols = codex_profile_columns()?;
    let target_config = cols
        .iter()
        .find(|c| c.id == change.target_install_id)
        .map(|c| c.home.join("config.toml"))
        .ok_or_else(|| format!("Unknown Codex profile: {}", change.target_install_id))?;

    if change.wants {
        let value = if let Some(src) = &change.source_install_id {
            let p = cols
                .iter()
                .find(|c| &c.id == src)
                .map(|c| c.home.join("config.toml"))
                .ok_or_else(|| format!("Unknown source: {src}"))?;
            read_codex_prefs_at(&p).get(key).cloned()
        } else {
            cols.iter()
                .filter(|c| c.id != change.target_install_id)
                .find_map(|c| read_codex_prefs_at(&c.home.join("config.toml")).get(key).cloned())
        }
        .ok_or_else(|| format!("No account sets \"{key}\" to copy from."))?;
        // No-op if already equal.
        if read_codex_prefs_at(&target_config).get(key) == Some(&value) {
            return Ok(false);
        }
        write_codex_pref_at(&target_config, key, &value)?;
        Ok(true)
    } else {
        remove_codex_pref_at(&target_config, key)
    }
}

fn write_codex_pref_at(path: &Path, key: &str, value: &serde_json::Value) -> Result<(), String> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("Parse {}: {e}", path.display()))?;
    let tv = json_to_toml_value(value)
        .ok_or_else(|| "Unsupported preference value type.".to_string())?;
    doc[key] = toml_edit::Item::Value(tv);
    write_string_atomically(path, &doc.to_string())
}

fn remove_codex_pref_at(path: &Path, key: &str) -> Result<bool, String> {
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(_) => return Ok(false),
    };
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("Parse {}: {e}", path.display()))?;
    let removed = doc.as_table_mut().remove(key).is_some();
    if removed {
        write_string_atomically(path, &doc.to_string())?;
    }
    Ok(removed)
}

/// Undo a directory symlink at `target`: remove it and copy back the real
/// content it pointed at. Generic version of make_codex_sessions_independent.
fn make_dir_symlink_independent(target: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let real = fs::read_link(target).ok().map(|link| {
                if link.is_absolute() {
                    link
                } else {
                    target.parent().unwrap_or(Path::new("/")).join(link)
                }
            });
            remove_path(target)?;
            if let Some(real) = real {
                if real.is_dir() {
                    copy_dir_recursive(&real, target)?;
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Share state for ANY symlink-mode kind, computed from the cells' actual
/// on-disk paths. A present cell is "shared" when it's in a symlink relationship
/// (either direction) with at least one other present cell — i.e. it links into
/// a sibling, OR a sibling links into it. This is the bidirectional rule that
/// correctly reads the real-source + single-symlink topology that every apply_*
/// function creates; compute_row_states' old digest-count rule (">=2 cells with
/// the same link digest") missed it, leaving genuinely-shared profiles showing
/// as "independent". Inputs are per-cell paths + present flags, parallel arrays.
fn symlink_share_states(paths: &[PathBuf], present: &[bool]) -> Vec<&'static str> {
    (0..paths.len())
        .map(|i| {
            if !present[i] {
                return "absent";
            }
            let linked = (0..paths.len()).any(|j| {
                j != i
                    && present[j]
                    && (path_points_to(&paths[i], &paths[j])
                        || path_points_to(&paths[j], &paths[i]))
            });
            if linked {
                "shared"
            } else {
                "independent"
            }
        })
        .collect()
}

/// Codex skills: per-profile <home>/skills dirs. Shareable BETWEEN Codex
/// profiles via symlink (same model as Claude skills).
pub fn list_codex_skills_library() -> Result<Vec<LibraryRow>, String> {
    let cols = codex_profile_columns()?;
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for col in &cols {
        let dir = col.home.join(SKILLS_SUBDIR);
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with('.') {
                    continue;
                }
                let is_link = fs::symlink_metadata(e.path())
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if e.path().is_dir() || is_link {
                    names.insert(n);
                }
            }
        }
    }
    let mut rows = Vec::new();
    for name in names {
        let paths: Vec<PathBuf> = cols
            .iter()
            .map(|c| c.home.join(SKILLS_SUBDIR).join(&name))
            .collect();
        let present: Vec<bool> = paths
            .iter()
            .map(|p| {
                p.is_dir()
                    || fs::symlink_metadata(p)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
            })
            .collect();
        let states = symlink_share_states(&paths, &present);
        let cells = cols
            .iter()
            .enumerate()
            .map(|(i, col)| LibraryCell {
                install_id: col.id.clone(),
                install_name: col.label.clone(),
                data_dir: col.home.join(SKILLS_SUBDIR).to_string_lossy().to_string(),
                kind: col.kind.clone(),
                state: states[i].to_string(),
                present: present[i],
                detail: None,
                digest: None,
                link_target_digest: symlink_target_digest(&paths[i]),
            })
            .collect();
        rows.push(LibraryRow {
            id: name.clone(),
            label: name,
            description: None,
            cells,
            interactive: true,
            group: Some("Codex skills".into()),
        });
    }
    Ok(rows)
}

/// Codex MCP: per-profile config.toml [mcp_servers]. Shareable BETWEEN Codex
/// profiles via copy-with-transform (copy semantics, like cross-tool MCP).
pub fn list_codex_mcp_library() -> Result<Vec<LibraryRow>, String> {
    let cols = codex_profile_columns()?;
    let maps: Vec<BTreeMap<String, serde_json::Value>> = cols
        .iter()
        .map(|c| read_codex_mcp_at(&c.home.join("config.toml")))
        .collect();
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for m in &maps {
        names.extend(m.keys().cloned());
    }
    let mut rows = Vec::new();
    for name in names {
        let cells = cols
            .iter()
            .enumerate()
            .map(|(ci, col)| {
                let server = maps[ci].get(&name);
                LibraryCell {
                    install_id: col.id.clone(),
                    install_name: col.label.clone(),
                    data_dir: col.home.join("config.toml").to_string_lossy().to_string(),
                    kind: col.kind.clone(),
                    state: String::new(),
                    present: server.is_some(),
                    detail: server.map(mcp_summary),
                    digest: server.map(mcp_value_digest),
                    link_target_digest: None,
                }
            })
            .collect();
        let mut row = LibraryRow {
            id: name.clone(),
            label: name,
            description: None,
            cells,
            interactive: true,
            group: Some("Codex MCP".into()),
        };
        compute_row_states(&mut row);
        rows.push(row);
    }
    Ok(rows)
}

/// Share a skill BETWEEN two Codex profiles by symlinking it into the target's
/// <home>/skills. Non-destructive: refuses to clobber a real folder / foreign
/// symlink (mirrors apply_skill_share for the cross-tool case).
fn apply_codex_skill_share(change: &LibraryCellChange) -> Result<bool, String> {
    let name = &change.row_id;
    let target_home = codex_home_for_column(&change.target_install_id)?
        .ok_or_else(|| format!("Unknown Codex profile: {}", change.target_install_id))?;
    let target_dir = target_home.join(SKILLS_SUBDIR);
    let target_link = target_dir.join(name);

    if change.wants {
        let source_home = if let Some(src) = &change.source_install_id {
            codex_home_for_column(src)?.ok_or_else(|| format!("Unknown source: {src}"))?
        } else {
            let rows = list_codex_skills_library()?;
            let row = rows
                .into_iter()
                .find(|r| r.id == *name)
                .ok_or_else(|| format!("Skill {name} not found"))?;
            let src_id = row
                .cells
                .iter()
                .find(|c| c.install_id != change.target_install_id && c.present)
                .ok_or_else(|| "No Codex profile holds this skill to share from.".to_string())?
                .install_id
                .clone();
            codex_home_for_column(&src_id)?.ok_or_else(|| "Source resolution failed".to_string())?
        };
        let source_path = source_home.join(SKILLS_SUBDIR).join(name);
        if !source_path.is_dir() && fs::symlink_metadata(&source_path).is_err() {
            return Err(format!("Source skill \"{name}\" doesn't exist."));
        }
        if path_points_to(&target_link, &source_path) {
            return Ok(false);
        }
        if fs::symlink_metadata(&target_link).is_ok() {
            return Err(format!(
                "\"{name}\" already exists in the target and isn't a Claudex link — remove it there first."
            ));
        }
        fs::create_dir_all(&target_dir).map_err(|e| format!("Create {}: {e}", target_dir.display()))?;
        symlink_path(&source_path, &target_link)?;
        Ok(true)
    } else {
        match fs::symlink_metadata(&target_link) {
            Ok(meta) if meta.file_type().is_symlink() => {
                remove_path(&target_link)?;
                Ok(true)
            }
            Ok(_) => Err(format!(
                "\"{name}\" in the target is a real folder, not a Claudex link — leaving it untouched."
            )),
            Err(_) => Ok(false),
        }
    }
}

/// Copy an MCP server BETWEEN two Codex profiles' config.toml. Copy semantics
/// with collision-refuse (different config) and no-op (identical).
fn apply_codex_mcp_share(change: &LibraryCellChange) -> Result<bool, String> {
    let name = &change.row_id;
    let target_home = codex_home_for_column(&change.target_install_id)?
        .ok_or_else(|| format!("Unknown Codex profile: {}", change.target_install_id))?;
    let target_config = target_home.join("config.toml");

    if change.wants {
        let source_home = if let Some(src) = &change.source_install_id {
            codex_home_for_column(src)?.ok_or_else(|| format!("Unknown source: {src}"))?
        } else {
            let mut found = None;
            for c in codex_profile_columns()? {
                if c.id == change.target_install_id {
                    continue;
                }
                if read_codex_mcp_at(&c.home.join("config.toml")).contains_key(name) {
                    found = Some(c.home);
                    break;
                }
            }
            found.ok_or_else(|| "No Codex profile holds this MCP server to copy from.".to_string())?
        };
        let server = read_codex_mcp_at(&source_home.join("config.toml"))
            .get(name)
            .cloned()
            .ok_or_else(|| format!("\"{name}\" isn't defined in the source profile."))?;
        if let Some(existing) = read_codex_mcp_at(&target_config).get(name) {
            if mcp_value_digest(existing) == mcp_value_digest(&server) {
                return Ok(false);
            }
            return Err(format!(
                "\"{name}\" already exists in the target with a different config — remove it there first."
            ));
        }
        write_codex_mcp_server_at(&target_config, name, &server)?;
        Ok(true)
    } else {
        remove_codex_mcp_server_at(&target_config, name)
    }
}

// ===========================================================================
// Cross-tool MCP (Share tab) — Claude Code (~/.claude.json `mcpServers`, JSON)
// <-> Codex (~/.codex/config.toml `[mcp_servers]`, TOML). Copy-mode with a
// JSON<->TOML transform; only stdio (command-based) servers are cross-tool
// shareable (remote/SSE Claude servers have no Codex equivalent and are
// skipped). Non-destructive: a name collision with a *different* config on the
// target refuses rather than clobbering.
// ===========================================================================

const MCP_CROSS_CLAUDE_ID: &str = "claude:code";

fn claude_code_config_path() -> Result<PathBuf, String> {
    // Claude Code keeps user-scope MCP servers in ~/.claude.json under the
    // top-level `mcpServers` object.
    Ok(home_dir()?.join(".claude.json"))
}

/// Atomic string write (temp + rename on the same fs). Sibling of
/// `write_json_atomically` for non-JSON (TOML) targets.
fn write_string_atomically(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create {}: {e}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("Invalid path: {}", path.display()))?;
    let tmp = path.with_file_name(format!(".{file_name}.tmp"));
    fs::write(&tmp, contents).map_err(|e| format!("Write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("Rename {} -> {}: {e}", tmp.display(), path.display()))
}

/// Recursively convert a toml_edit value into serde_json (Codex MCP -> IR).
fn toml_value_to_json(v: &toml_edit::Value) -> serde_json::Value {
    use serde_json::Value as J;
    use toml_edit::Value as T;
    match v {
        T::String(s) => J::String(s.value().clone()),
        T::Integer(i) => J::Number((*i.value()).into()),
        T::Float(f) => serde_json::Number::from_f64(*f.value())
            .map(J::Number)
            .unwrap_or(J::Null),
        T::Boolean(b) => J::Bool(*b.value()),
        T::Datetime(d) => J::String(d.value().to_string()),
        T::Array(a) => J::Array(a.iter().map(toml_value_to_json).collect()),
        T::InlineTable(t) => {
            let mut map = serde_json::Map::new();
            for (k, val) in t.iter() {
                map.insert(k.to_string(), toml_value_to_json(val));
            }
            J::Object(map)
        }
    }
}

fn toml_item_to_json(item: &toml_edit::Item) -> serde_json::Value {
    use serde_json::Value as J;
    use toml_edit::Item;
    match item {
        Item::Value(v) => toml_value_to_json(v),
        Item::Table(t) => {
            let mut map = serde_json::Map::new();
            for (k, v) in t.iter() {
                map.insert(k.to_string(), toml_item_to_json(v));
            }
            J::Object(map)
        }
        Item::ArrayOfTables(arr) => J::Array(
            arr.iter()
                .map(|t| {
                    let mut map = serde_json::Map::new();
                    for (k, v) in t.iter() {
                        map.insert(k.to_string(), toml_item_to_json(v));
                    }
                    J::Object(map)
                })
                .collect(),
        ),
        Item::None => J::Null,
    }
}

/// Convert a (scalar/array) JSON value into a toml_edit value. Objects are
/// handled one level up by `json_object_to_toml_table` so they render as
/// nested `[table]`s (e.g. `env`) rather than inline.
fn json_to_toml_value(v: &serde_json::Value) -> Option<toml_edit::Value> {
    use serde_json::Value as J;
    use toml_edit::Value as T;
    Some(match v {
        J::Null => return None,
        J::Bool(b) => T::from(*b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                T::from(i)
            } else if let Some(f) = n.as_f64() {
                T::from(f)
            } else {
                return None;
            }
        }
        J::String(s) => T::from(s.clone()),
        J::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for x in a {
                if let Some(tv) = json_to_toml_value(x) {
                    arr.push(tv);
                }
            }
            T::Array(arr)
        }
        J::Object(o) => {
            let mut it = toml_edit::InlineTable::new();
            for (k, val) in o {
                if let Some(tv) = json_to_toml_value(val) {
                    it.insert(k, tv);
                }
            }
            T::InlineTable(it)
        }
    })
}

/// Build a real TOML table from a JSON object so nested objects (`env`) become
/// idiomatic `[mcp_servers.<name>.env]` sub-tables.
fn json_object_to_toml_table(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> toml_edit::Table {
    use serde_json::Value as J;
    let mut table = toml_edit::Table::new();
    for (k, v) in obj {
        match v {
            J::Object(inner) => {
                table.insert(k, toml_edit::Item::Table(json_object_to_toml_table(inner)));
            }
            other => {
                if let Some(tv) = json_to_toml_value(other) {
                    table.insert(k, toml_edit::Item::Value(tv));
                }
            }
        }
    }
    table
}

/// Order-independent serialization for digesting two MCP configs across the
/// JSON/TOML boundary (object keys sorted; arrays kept ordered).
fn canonical_json_string(v: &serde_json::Value) -> String {
    use serde_json::Value as J;
    match v {
        J::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .iter()
                .map(|k| format!("{:?}:{}", k, canonical_json_string(&o[*k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        J::Array(a) => {
            let inner: Vec<String> = a.iter().map(canonical_json_string).collect();
            format!("[{}]", inner.join(","))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn mcp_value_digest(v: &serde_json::Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    canonical_json_string(v).hash(&mut h);
    format!("{:016x}", h.finish())
}

fn mcp_summary(v: &serde_json::Value) -> String {
    let cmd = v.get("command").and_then(|c| c.as_str()).unwrap_or("");
    let arg0 = v
        .get("args")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.as_str())
        .unwrap_or("");
    format!("{cmd} {arg0}").trim().to_string()
}

/// Read Claude Code's user-scope stdio MCP servers (name -> JSON config).
fn read_claude_code_mcp() -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    let path = match claude_code_config_path() {
        Ok(p) => p,
        Err(_) => return out,
    };
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return out,
    };
    let val: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return out,
    };
    if let Some(servers) = val.get("mcpServers").and_then(|v| v.as_object()) {
        for (name, cfg) in servers {
            // stdio only — remote (sse/http/url) servers have no Codex analog.
            if cfg.get("command").and_then(|c| c.as_str()).is_some() {
                out.insert(name.clone(), cfg.clone());
            }
        }
    }
    out
}

/// Read stdio MCP servers (name -> JSON config, transformed from TOML) from a
/// specific Codex config.toml. Used for both the default ~/.codex and each
/// managed profile's ~/.codex-<name>.
fn read_codex_mcp_at(path: &Path) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(_) => return out,
    };
    let doc: toml_edit::ImDocument<String> = match raw.parse() {
        Ok(d) => d,
        Err(_) => return out,
    };
    if let Some(servers) = doc.get("mcp_servers").and_then(|i| i.as_table()) {
        for (name, item) in servers.iter() {
            let json = toml_item_to_json(item);
            if json.get("command").and_then(|c| c.as_str()).is_some() {
                out.insert(name.to_string(), json);
            }
        }
    }
    out
}

/// The default ~/.codex MCP servers (cross-tool Claude<->Codex sharing).
fn read_codex_mcp() -> BTreeMap<String, serde_json::Value> {
    match codex_config_path() {
        Ok(p) => read_codex_mcp_at(&p),
        Err(_) => BTreeMap::new(),
    }
}

fn mcp_cross_cell(
    install_id: &str,
    install_name: &str,
    data_dir: &str,
    kind: &str,
    server: Option<&serde_json::Value>,
) -> LibraryCell {
    LibraryCell {
        install_id: install_id.to_string(),
        install_name: install_name.to_string(),
        data_dir: data_dir.to_string(),
        kind: kind.to_string(),
        state: String::new(),
        present: server.is_some(),
        detail: server.map(mcp_summary),
        digest: server.map(mcp_value_digest),
        link_target_digest: None,
    }
}

/// The cross-tool MCP matrix: one row per server name, two columns
/// (Claude Code, Codex). Copy semantics — `copied` when both sides hold an
/// identical config, `diverged` when they differ.
pub fn list_mcp_cross_library() -> Result<Vec<LibraryRow>, String> {
    let claude = read_claude_code_mcp();
    let codex = read_codex_mcp();
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    names.extend(claude.keys().cloned());
    names.extend(codex.keys().cloned());

    let mut rows = Vec::new();
    for name in names {
        let cells = vec![
            mcp_cross_cell(
                MCP_CROSS_CLAUDE_ID,
                "Claude Code",
                "~/.claude.json",
                "default",
                claude.get(&name),
            ),
            mcp_cross_cell(
                CODEX_SKILLS_GLOBAL_ID,
                "Codex",
                "~/.codex/config.toml",
                "codex",
                codex.get(&name),
            ),
        ];
        let mut row = LibraryRow {
            id: name.clone(),
            label: name,
            description: None,
            cells,
            interactive: true,
            group: Some("Claude Code + Codex MCP".into()),
        };
        compute_row_states(&mut row); // copy semantics, no symlinks
        rows.push(row);
    }
    Ok(rows)
}

fn write_claude_code_mcp_server(name: &str, server: &serde_json::Value) -> Result<(), String> {
    let path = claude_code_config_path()?;
    let raw = fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("Parse ~/.claude.json: {e}"))?;
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "~/.claude.json is not a JSON object.".to_string())?;
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| "mcpServers in ~/.claude.json is not an object.".to_string())?;
    servers.insert(name.to_string(), server.clone());
    write_json_atomically(&path, &root)
}

fn remove_claude_code_mcp_server(name: &str) -> Result<bool, String> {
    let path = claude_code_config_path()?;
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return Ok(false),
    };
    let mut root: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("Parse ~/.claude.json: {e}"))?;
    let removed = root
        .as_object_mut()
        .and_then(|o| o.get_mut("mcpServers"))
        .and_then(|s| s.as_object_mut())
        .map(|servers| servers.remove(name).is_some())
        .unwrap_or(false);
    if removed {
        write_json_atomically(&path, &root)?;
    }
    Ok(removed)
}

fn write_codex_mcp_server_at(
    path: &Path,
    name: &str,
    server: &serde_json::Value,
) -> Result<(), String> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("Parse {}: {e}", path.display()))?;
    if doc.get("mcp_servers").is_none() {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        doc["mcp_servers"] = toml_edit::Item::Table(t);
    }
    let servers = doc["mcp_servers"]
        .as_table_mut()
        .ok_or_else(|| "mcp_servers in config.toml is not a table.".to_string())?;
    let obj = server
        .as_object()
        .ok_or_else(|| "MCP server config is not an object.".to_string())?;
    servers.insert(name, toml_edit::Item::Table(json_object_to_toml_table(obj)));
    write_string_atomically(path, &doc.to_string())
}

fn write_codex_mcp_server(name: &str, server: &serde_json::Value) -> Result<(), String> {
    write_codex_mcp_server_at(&codex_config_path()?, name, server)
}

fn remove_codex_mcp_server_at(path: &Path, name: &str) -> Result<bool, String> {
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(_) => return Ok(false),
    };
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("Parse {}: {e}", path.display()))?;
    let removed = doc
        .get_mut("mcp_servers")
        .and_then(|i| i.as_table_mut())
        .map(|t| t.remove(name).is_some())
        .unwrap_or(false);
    if removed {
        write_string_atomically(path, &doc.to_string())?;
    }
    Ok(removed)
}

fn remove_codex_mcp_server(name: &str) -> Result<bool, String> {
    remove_codex_mcp_server_at(&codex_config_path()?, name)
}

/// Apply a cross-tool MCP toggle. wants=true copies the server FROM the other
/// side INTO the target side (refusing a different-config collision); wants=
/// false removes it from the target.
fn apply_mcp_cross_share(change: &LibraryCellChange) -> Result<bool, String> {
    let name = &change.row_id;
    let target = change.target_install_id.as_str();
    let claude = read_claude_code_mcp();
    let codex = read_codex_mcp();

    if change.wants {
        match target {
            MCP_CROSS_CLAUDE_ID => {
                let server = codex.get(name).ok_or_else(|| {
                    format!("\"{name}\" isn't defined in Codex to copy from.")
                })?;
                if let Some(existing) = claude.get(name) {
                    if mcp_value_digest(existing) == mcp_value_digest(server) {
                        return Ok(false); // identical — no-op
                    }
                    return Err(format!(
                        "\"{name}\" already exists in Claude Code with a different config — remove it there first."
                    ));
                }
                write_claude_code_mcp_server(name, server)?;
                Ok(true)
            }
            CODEX_SKILLS_GLOBAL_ID => {
                let server = claude.get(name).ok_or_else(|| {
                    format!("\"{name}\" isn't defined in Claude Code to copy from.")
                })?;
                if let Some(existing) = codex.get(name) {
                    if mcp_value_digest(existing) == mcp_value_digest(server) {
                        return Ok(false);
                    }
                    return Err(format!(
                        "\"{name}\" already exists in Codex with a different config — remove it there first."
                    ));
                }
                write_codex_mcp_server(name, server)?;
                Ok(true)
            }
            other => Err(format!("Unknown MCP column: {other}")),
        }
    } else {
        match target {
            MCP_CROSS_CLAUDE_ID => remove_claude_code_mcp_server(name),
            CODEX_SKILLS_GLOBAL_ID => remove_codex_mcp_server(name),
            other => Err(format!("Unknown MCP column: {other}")),
        }
    }
}

// ===========================================================================
// Memory (Share tab) — the agent's memory/instruction Markdown file. Claude Code
// reads <config_dir>/CLAUDE.md, Codex reads <CODEX_HOME>/AGENTS.md. It's just
// Markdown, so it symlink-shares exactly like a skill — within Claude accounts,
// within Codex accounts, AND across the two. The filename differs per tool, but
// a symlink's BASENAME is fixed by its location (CLAUDE.md or AGENTS.md) while
// its TARGET is the source file, so CLAUDE.md <-> AGENTS.md bridges cleanly. One
// matrix, every account a column. Column ids are namespaced ("claude-code:<id>"
// / "codex:<id>") because Claude-code and Codex both use "default"/"profile:*".
// ===========================================================================

const MEMORY_CLAUDE_FILE: &str = "CLAUDE.md";
const MEMORY_CODEX_FILE: &str = "AGENTS.md";
const MEMORY_CLAUDE_PREFIX: &str = "claude-code:";
const MEMORY_CODEX_PREFIX: &str = "codex:";

struct MemoryCol {
    id: String,
    label: String,
    kind: String,
    path: PathBuf,
}

/// Within-Claude memory columns: one CLAUDE.md per Claude code install (plain ids).
fn claude_memory_columns() -> Result<Vec<MemoryCol>, String> {
    let mut cols = Vec::new();
    for inst in list_code_installs()? {
        let label = if inst.kind == "default" {
            "Default ~/.claude".to_string()
        } else {
            inst.name.clone()
        };
        cols.push(MemoryCol {
            id: inst.id.clone(),
            label,
            kind: "claude".to_string(),
            path: PathBuf::from(inst.config_dir).join(MEMORY_CLAUDE_FILE),
        });
    }
    Ok(cols)
}

/// Within-Codex memory columns: one AGENTS.md per Codex home (plain ids).
fn codex_memory_columns() -> Result<Vec<MemoryCol>, String> {
    Ok(codex_profile_columns()?
        .into_iter()
        .map(|c| MemoryCol {
            id: c.id,
            label: c.label,
            kind: "codex".to_string(),
            path: c.home.join(MEMORY_CODEX_FILE),
        })
        .collect())
}

/// Cross-platform memory columns: Claude CLAUDE.md + Codex AGENTS.md, ids
/// namespaced (claude-code:/codex:) because both worlds use "default"/"profile:*".
fn memory_columns() -> Result<Vec<MemoryCol>, String> {
    let mut cols = Vec::new();
    for c in claude_memory_columns()? {
        cols.push(MemoryCol {
            id: format!("{MEMORY_CLAUDE_PREFIX}{}", c.id),
            label: c.label,
            kind: c.kind,
            path: c.path,
        });
    }
    for c in codex_memory_columns()? {
        cols.push(MemoryCol {
            id: format!("{MEMORY_CODEX_PREFIX}{}", c.id),
            label: format!("Codex {}", c.label),
            kind: c.kind,
            path: c.path,
        });
    }
    Ok(cols)
}

/// A path is a "real" memory file (regular file, not a symlink).
fn is_real_file(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

/// A path is present as a memory file (real file OR a symlink to one).
fn memory_present(p: &Path) -> bool {
    is_real_file(p)
        || fs::symlink_metadata(p)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
}

/// The Memory matrix for a given column set: a single row, one column per
/// account. Symlink-share, bidirectional state via symlink_share_states.
fn memory_matrix_rows(cols: &[MemoryCol]) -> Vec<LibraryRow> {
    let paths: Vec<PathBuf> = cols.iter().map(|c| c.path.clone()).collect();
    let present: Vec<bool> = paths.iter().map(|p| memory_present(p)).collect();
    let states = symlink_share_states(&paths, &present);
    let cells = cols
        .iter()
        .enumerate()
        .map(|(i, c)| LibraryCell {
            install_id: c.id.clone(),
            install_name: c.label.clone(),
            data_dir: c.path.to_string_lossy().to_string(),
            kind: c.kind.clone(),
            state: states[i].to_string(),
            present: present[i],
            // Always name the file so an absent cell still reads
            // "CLAUDE.md · not created" (self-describing + clickable to
            // create) instead of a blank "—". This is what makes Memory
            // visible on a fresh account.
            detail: {
                let filename = if c.kind == "codex" {
                    MEMORY_CODEX_FILE
                } else {
                    MEMORY_CLAUDE_FILE
                };
                Some(if present[i] {
                    filename.to_string()
                } else {
                    format!("{filename} · not created")
                })
            },
            digest: None,
            link_target_digest: symlink_target_digest(&paths[i]),
        })
        .collect();
    vec![LibraryRow {
        id: "memory".to_string(),
        label: "Agent memory".to_string(),
        description: Some("Markdown — shares like a skill".to_string()),
        cells,
        interactive: true,
        group: None,
    }]
}

/// Share the memory file within a column set: symlink the target account's
/// memory file at the source's. The target link keeps its own basename
/// (CLAUDE.md / AGENTS.md) and points at the source, so cross-tool sharing
/// bridges the filename gap. Non-destructive: refuses to clobber a real file.
fn apply_memory_share_in(
    cols: &[MemoryCol],
    change: &LibraryCellChange,
) -> Result<bool, String> {
    let path_for = |id: &str| cols.iter().find(|c| c.id == id).map(|c| c.path.clone());
    let target_link = path_for(&change.target_install_id)
        .ok_or_else(|| format!("Unknown memory column: {}", change.target_install_id))?;

    if change.wants {
        let source_path = if let Some(src) = &change.source_install_id {
            path_for(src).ok_or_else(|| format!("Unknown source: {src}"))?
        } else {
            // Prefer a real-file source over a symlink (avoids symlink chains).
            cols.iter()
                .find(|c| c.id != change.target_install_id && is_real_file(&c.path))
                .or_else(|| {
                    cols.iter()
                        .find(|c| c.id != change.target_install_id && memory_present(&c.path))
                })
                .ok_or_else(|| "No account holds a memory file to share from.".to_string())?
                .path
                .clone()
        };
        if !memory_present(&source_path) {
            return Err("Source memory file doesn't exist.".to_string());
        }
        if path_points_to(&target_link, &source_path) {
            return Ok(false);
        }
        if fs::symlink_metadata(&target_link).is_ok() {
            return Err(format!(
                "A memory file already exists at the target ({}) and isn't a Claudex link — remove it there first.",
                target_link.display()
            ));
        }
        if let Some(parent) = target_link.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Create {}: {e}", parent.display()))?;
        }
        symlink_path(&source_path, &target_link)?;
        Ok(true)
    } else {
        match fs::symlink_metadata(&target_link) {
            Ok(meta) if meta.file_type().is_symlink() => {
                remove_path(&target_link)?;
                Ok(true)
            }
            Ok(_) => Err(
                "The target memory file is real, not a Claudex link — leaving it untouched."
                    .to_string(),
            ),
            Err(_) => Ok(false),
        }
    }
}

/// Cross-platform memory (Share tab): CLAUDE.md ↔ AGENTS.md across all accounts.
pub fn list_memory_library() -> Result<Vec<LibraryRow>, String> {
    Ok(memory_matrix_rows(&memory_columns()?))
}
fn apply_memory_share(change: &LibraryCellChange) -> Result<bool, String> {
    apply_memory_share_in(&memory_columns()?, change)
}

/// Collapse a git-worktree cwd to its parent project. Claude/gstack worktrees
/// live at `<project>/.claude/worktrees/<name>`, each with its OWN
/// ~/.claude/projects entry — without this they'd each surface as a separate
/// project row of random codenames. Returns the cwd unchanged when not a worktree.
fn worktree_root(cwd: &str) -> &str {
    match cwd.find("/.claude/worktrees/") {
        Some(i) => &cwd[..i],
        None => cwd,
    }
}

/// Read the real working directory recorded inside a project's newest session
/// transcript. The `~/.claude/projects/<id>` directory name is a LOSSY encoding
/// of the cwd (both `/` and `.` collapse to `-`), so it can't be decoded back —
/// the authoritative cwd lives in the `cwd` field of the JSONL events.
fn project_cwd_from_sessions(project_dir: &Path) -> Option<String> {
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in fs::read_dir(project_dir).ok()?.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(m) = p.metadata().ok().and_then(|md| md.modified().ok()) else {
            continue;
        };
        if newest.as_ref().map_or(true, |(t, _)| m > *t) {
            newest = Some((m, p));
        }
    }
    let (_, file) = newest?;
    let f = fs::File::open(&file).ok()?;
    let mut reader = std::io::BufReader::new(f);
    let mut line = String::new();
    // The cwd is recorded on the first events; scan a few lines, not the whole
    // (potentially multi-MB) transcript.
    for _ in 0..8 {
        line.clear();
        if std::io::BufRead::read_line(&mut reader, &mut line).ok()? == 0 {
            break;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if let Some(cwd) = v.get("cwd").and_then(|c| c.as_str()) {
                if !cwd.is_empty() {
                    return Some(cwd.to_string());
                }
            }
        }
    }
    None
}

/// Project-level memory: a `CLAUDE.md` at a project's working directory (the
/// common case — most CLAUDE.md live in repos, not in `~/.claude`). One row per
/// project that actually has a CLAUDE.md, a cell present under every account that
/// has worked in that project. Browse/edit only (the file is shared by path, so
/// there's nothing to symlink-share).
fn claude_project_memory_rows() -> Vec<LibraryRow> {
    let installs = match list_code_installs() {
        Ok(i) => i,
        Err(_) => return Vec::new(),
    };
    let mut by_cwd: std::collections::BTreeMap<String, std::collections::HashSet<String>> =
        std::collections::BTreeMap::new();
    for inst in &installs {
        let projects = PathBuf::from(&inst.config_dir).join("projects");
        let Ok(rd) = fs::read_dir(&projects) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            if let Some(cwd) = project_cwd_from_sessions(&p) {
                // Skip ephemeral agent worktrees — their CLAUDE.md just mirrors
                // the parent repo's, so they'd flood the list with near-dupes.
                if cwd.contains("/.claude/worktrees/") {
                    continue;
                }
                by_cwd.entry(cwd).or_default().insert(inst.id.clone());
            }
        }
    }
    let mut rows = Vec::new();
    for (cwd, ids) in by_cwd {
        let memfile = PathBuf::from(&cwd).join(MEMORY_CLAUDE_FILE);
        if !memfile.is_file() {
            continue; // only surface projects that actually have memory
        }
        let label = Path::new(&cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| cwd.clone());
        let memfile_str = memfile.to_string_lossy().to_string();
        let cells = installs
            .iter()
            .map(|inst| {
                let present = ids.contains(&inst.id);
                LibraryCell {
                    install_id: inst.id.clone(),
                    install_name: inst.name.clone(),
                    data_dir: memfile_str.clone(),
                    kind: inst.kind.clone(),
                    state: if present { "independent" } else { "absent" }.to_string(),
                    present,
                    detail: Some("CLAUDE.md".to_string()),
                    digest: None,
                    link_target_digest: None,
                }
            })
            .collect();
        rows.push(LibraryRow {
            id: format!("projmem:{cwd}"),
            label,
            description: Some(cwd.clone()),
            cells,
            interactive: false,
            group: Some("PROJECT MEMORY".to_string()),
        });
    }
    rows
}

/// Within-Claude memory (Claude tab): the global `~/.claude/CLAUDE.md` per
/// account, plus every project-level `CLAUDE.md` the accounts have touched.
pub fn list_claude_memory_library() -> Result<Vec<LibraryRow>, String> {
    let mut rows = memory_matrix_rows(&claude_memory_columns()?);
    if let Some(first) = rows.first_mut() {
        first.group = Some("GLOBAL · ~/.claude".to_string());
    }
    rows.extend(claude_project_memory_rows());
    Ok(rows)
}
fn apply_claude_memory_share(change: &LibraryCellChange) -> Result<bool, String> {
    apply_memory_share_in(&claude_memory_columns()?, change)
}

/// Within-Codex memory (Codex tab): AGENTS.md across Codex accounts.
pub fn list_codex_memory_library() -> Result<Vec<LibraryRow>, String> {
    Ok(memory_matrix_rows(&codex_memory_columns()?))
}
fn apply_codex_memory_share(change: &LibraryCellChange) -> Result<bool, String> {
    apply_memory_share_in(&codex_memory_columns()?, change)
}

pub fn apply_library_change(
    kind: String,
    change: LibraryCellChange,
) -> Result<bool, String> {
    if kind == "skills" || kind == "claude_skills" {
        return apply_skill_share(&change);
    }
    if kind == "claude_sessions" {
        return apply_claude_sessions_share(&change);
    }
    if kind == "codex_preferences" {
        return apply_codex_preferences_share(&change);
    }
    if kind == "sessions_cross" {
        return Err(
            "Cross-tool sessions are import/export only — use the Import/Export action on a row."
                .to_string(),
        );
    }
    if kind == "mcp_cross" {
        return apply_mcp_cross_share(&change);
    }
    if kind == "codex_skills" {
        return apply_codex_skill_share(&change);
    }
    if kind == "codex_mcp" {
        return apply_codex_mcp_share(&change);
    }
    if kind == "codex_sessions" {
        return apply_codex_sessions_share(&change);
    }
    if kind == "memory_cross" {
        return apply_memory_share(&change);
    }
    if kind == "memory" {
        return apply_claude_memory_share(&change);
    }
    if kind == "codex_memory" {
        return apply_codex_memory_share(&change);
    }
    let installs = list_desktop_installs()?;
    let target = installs
        .iter()
        .find(|i| i.id == change.target_install_id)
        .ok_or_else(|| format!("Target profile {} not found", change.target_install_id))?
        .clone();

    // Pick a source: explicit, or first present sibling.
    let source = if let Some(explicit) = &change.source_install_id {
        installs
            .iter()
            .find(|i| &i.id == explicit)
            .cloned()
            .ok_or_else(|| format!("Source profile {explicit} not found"))?
    } else {
        // Need to know which profiles currently have the row's content.
        let rows = match kind.as_str() {
            "extensions" => list_extensions_library_grid()?,
            "mcp_servers" => list_mcp_library()?,
            "cowork_skills" => list_cowork_skills_library()?,
            "preferences" => list_preferences_library()?,
            "code_history" => list_code_history_library()?,
            "cowork_sessions" => list_cowork_sessions_library()?,
            _ => return Err(format!("Unknown library kind: {kind}")),
        };
        let row = rows
            .into_iter()
            .find(|r| r.id == change.row_id)
            .ok_or_else(|| format!("Row {} not found in {kind}", change.row_id))?;
        // Prefer the default install if it has it; else any other present cell.
        let pick = row
            .cells
            .iter()
            .find(|c| c.install_id != change.target_install_id && c.present && c.kind == "default")
            .or_else(|| {
                row.cells
                    .iter()
                    .find(|c| c.install_id != change.target_install_id && c.present)
            });
        match pick {
            Some(c) => installs
                .iter()
                .find(|i| i.id == c.install_id)
                .cloned()
                .ok_or_else(|| "Source resolution failed".to_string())?,
            None if !change.wants => {
                // Nothing to copy from but user wants to remove — use any
                // other profile as a placeholder source; the OFF branch of
                // each pair function only reads target.
                installs
                    .iter()
                    .find(|i| i.id != change.target_install_id)
                    .cloned()
                    .ok_or_else(|| {
                        "Need at least two profiles for sharing operations.".to_string()
                    })?
            }
            None => {
                return Err("No profile holds this item; nothing to copy from.".into());
            }
        }
    };

    let source_dir = PathBuf::from(&source.data_dir);
    let target_dir = PathBuf::from(&target.data_dir);

    match kind.as_str() {
        "extensions" => {
            if change.wants {
                copy_extension_between_dirs(&source_dir, &target_dir, &change.row_id)?;
                Ok(true)
            } else {
                // Reuse the pair API in "make independent" mode.
                set_pair_extension_shared(&source_dir, &target_dir, &change.row_id, false)
            }
        }
        "mcp_servers" => {
            set_pair_mcp_server_copied(&source_dir, &target_dir, &change.row_id, change.wants)
        }
        "cowork_skills" => {
            set_pair_cowork_skill_shared(&source_dir, &target_dir, &change.row_id, change.wants)
        }
        "preferences" => {
            let colon = change.row_id.find(':').ok_or_else(|| {
                "Preference row id must be 'scope:key'".to_string()
            })?;
            let scope = &change.row_id[..colon];
            let key = &change.row_id[colon + 1..];
            set_pair_preference_copied(&source_dir, &target_dir, key, scope, change.wants)
        }
        "code_history" => {
            // Only the synthetic "__workspace__" row toggles the symlink —
            // per-cwd rows are browse-only and the frontend won't send them.
            if change.row_id != "__workspace__" {
                return Err(
                    "Per-project Code History rows are browse-only — toggle the “Whole workspace” row to share."
                        .into(),
                );
            }
            let summary = apply_pair_desktop_code_history(
                source.data_dir.clone(),
                target.data_dir.clone(),
                PairDesktopCodeHistoryChange { shared: change.wants },
            )?;
            Ok(summary.copied > 0)
        }
        "cowork_sessions" => {
            // Cowork agent-mode sessions aren't share-toggleable today — they
            // bind to the account at sub-directory level rather than a clean
            // symlink boundary. v1 keeps them strictly informational.
            Err("Cowork sessions are read-only in this version.".into())
        }
        other => Err(format!("Unknown library kind: {other}")),
    }
}

pub fn apply_library_changes(
    kind: String,
    changes: Vec<LibraryCellChange>,
) -> Result<CopySummary, String> {
    // Collapse a multi-cell "All sessions" share batch to one canonical source +
    // spokes BEFORE applying, so no second per-change auto-pick can observe a
    // freshly-created symlink and form a cycle (the data-loss guard). Strictly
    // gated on the two sessions kinds — every other kind passes through.
    let changes = if matches!(kind.as_str(), "codex_sessions" | "claude_sessions") {
        canonicalize_sessions_batch(&kind, changes)?
    } else {
        changes
    };
    let mut copied = 0;
    let mut skipped = 0;
    for change in changes {
        match apply_library_change(kind.clone(), change) {
            Ok(true) => copied += 1,
            Ok(false) => skipped += 1,
            Err(e) => return Err(e),
        }
    }
    Ok(CopySummary { copied, skipped })
}

// ----- Local session scanning -----
//
// Two storage trees hold per-conversation JSON files:
//
//   claude-code-sessions/<acct>/<org>/local_*.json    — Cowork "Code" panel
//   local-agent-mode-sessions/<acct>/<group>/local_*.json — Cowork agent mode
//
// Both files are flat JSON with the same surface: sessionId, cwd, title,
// model, createdAt, lastActivityAt, (sometimes) accountName/emailAddress.
// Parsing them gives the user a real-content view ("Investigate storage
// full issue · Opus 4.7 · 2h ago") instead of just aggregate counts.

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalSession {
    pub session_id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    /// Cowork agent mode uses a VM "processName" instead of a real cwd.
    pub process_name: Option<String>,
    pub model: Option<String>,
    pub created_at_ms: i64,
    pub last_activity_ms: i64,
    /// Surfaced from Cowork agent-mode session files when available — the
    /// only way to see the human-readable account on this profile without
    /// reading Local Storage / IndexedDB. Code-panel sessions don't carry it.
    pub account_name: Option<String>,
    pub email_address: Option<String>,
}

/// Read only the fields we care about — sessions can be hundreds of KB
/// because of systemPrompt + initialMessage, so streaming-parse + early-pick
/// would be ideal, but serde_json::Value is plenty fast at this scale (<200
/// files, <2MB total in practice).
fn parse_local_session(path: &Path) -> Option<LocalSession> {
    let raw = fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let session_id = v.get("sessionId").and_then(|x| x.as_str())?.to_string();
    let created = v
        .get("createdAt")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let last = v
        .get("lastActivityAt")
        .and_then(|x| x.as_i64())
        .unwrap_or(created);
    Some(LocalSession {
        session_id,
        title: v.get("title").and_then(|x| x.as_str()).map(String::from),
        cwd: v.get("cwd").and_then(|x| x.as_str()).map(String::from),
        process_name: v
            .get("processName")
            .and_then(|x| x.as_str())
            .map(String::from),
        model: v.get("model").and_then(|x| x.as_str()).map(String::from),
        created_at_ms: created,
        last_activity_ms: last,
        account_name: v
            .get("accountName")
            .and_then(|x| x.as_str())
            .map(String::from),
        email_address: v
            .get("emailAddress")
            .and_then(|x| x.as_str())
            .map(String::from),
    })
}

/// Walk every `local_*.json` under `root`, return parsed sessions.
fn scan_sessions_under(root: &Path) -> Vec<LocalSession> {
    let mut out = Vec::new();
    if let Ok(walker) = fs::read_dir(root) {
        for outer in walker.flatten() {
            let outer_path = outer.path();
            if !outer_path.is_dir() {
                continue;
            }
            // Skip skills-plugin/ — that's manifest+skills, not sessions.
            if outer_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s == "skills-plugin")
                .unwrap_or(false)
            {
                continue;
            }
            if let Ok(mid) = fs::read_dir(&outer_path) {
                for mid_e in mid.flatten() {
                    let mid_path = mid_e.path();
                    if !mid_path.is_dir() {
                        continue;
                    }
                    // Scan one level deeper for local_*.json
                    if let Ok(leaf) = fs::read_dir(&mid_path) {
                        for f in leaf.flatten() {
                            let p = f.path();
                            if p.file_name()
                                .and_then(|n| n.to_str())
                                .map(|s| s.starts_with("local_") && s.ends_with(".json"))
                                .unwrap_or(false)
                            {
                                if let Some(session) = parse_local_session(&p) {
                                    out.push(session);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn code_sessions_root(data_dir: &Path) -> PathBuf {
    data_dir.join(DESKTOP_CODE_SESSIONS_DIR)
}

fn cowork_sessions_root(data_dir: &Path) -> PathBuf {
    data_dir.join("local-agent-mode-sessions")
}

/// One scan, one cell. Encapsulates the per-profile aggregate for a given
/// row (cwd or processName).
fn build_session_cell(
    install: &DesktopInstall,
    sessions: &[LocalSession],
    link_target_digest: Option<String>,
) -> LibraryCell {
    let n = sessions.len();
    let last_activity = sessions.iter().map(|s| s.last_activity_ms).max().unwrap_or(0);
    let best_title = sessions
        .iter()
        .max_by_key(|s| s.last_activity_ms)
        .and_then(|s| s.title.clone());
    let detail = if n == 0 {
        None
    } else {
        let mut parts: Vec<String> = vec![format!(
            "{n} session{}",
            if n == 1 { "" } else { "s" }
        )];
        if let Some(t) = best_title {
            let trimmed = if t.len() > 36 { format!("{}…", &t[..35]) } else { t };
            parts.push(format!("“{trimmed}”"));
        }
        if last_activity > 0 {
            parts.push(humanize_ago(last_activity));
        }
        Some(parts.join(" · "))
    };
    LibraryCell {
        install_id: install.id.clone(),
        install_name: install.name.clone(),
        data_dir: install.data_dir.clone(),
        kind: install.kind.clone(),
        state: String::new(),
        present: n > 0,
        detail,
        // The "digest" we set here represents how many sessions exist (so
        // identical session counts across profiles look "copied"). Probably
        // not what we want — keep it None and let the symlink decide.
        digest: None,
        link_target_digest,
    }
}

// ----- Code History library — per-cwd matrix view -----
//
// Each Desktop profile has exactly one current workspace
// (`claude-code-sessions/<accountId>/<orgId>/`) and the share unit is
// "this profile's workspace IS that profile's workspace" via symlink.
// But the user thinks in *projects*, not workspaces — they want to know
// where they worked on `democra-ai`, where on `OpenAdvisor`. So we
// explode the workspace into one row per unique cwd, plus a leading
// "Workspace" row that carries the symlink-share state for the whole.

pub fn list_code_history_library() -> Result<Vec<LibraryRow>, String> {
    list_session_library(SessionKind::CodePanel)
}

pub fn list_cowork_sessions_library() -> Result<Vec<LibraryRow>, String> {
    list_session_library(SessionKind::CoworkAgent)
}

#[derive(Clone, Copy)]
enum SessionKind {
    CodePanel,
    CoworkAgent,
}

fn list_session_library(kind: SessionKind) -> Result<Vec<LibraryRow>, String> {
    let installs = list_desktop_installs()?;
    // (install, sessions, workspace_path). The workspace path is the share unit
    // for the code panel (per-account symlink target); Cowork agent mode has no
    // workspace symlink, so None.
    let mut per_install: Vec<(DesktopInstall, Vec<LocalSession>, Option<PathBuf>)> =
        Vec::with_capacity(installs.len());

    for install in installs {
        let data_dir = PathBuf::from(&install.data_dir);
        let root = match kind {
            SessionKind::CodePanel => code_sessions_root(&data_dir),
            SessionKind::CoworkAgent => cowork_sessions_root(&data_dir),
        };
        let sessions = scan_sessions_under(&root);
        let ws_path = match kind {
            SessionKind::CodePanel => read_workspace_identity(&data_dir)
                .unwrap_or(None)
                .map(|w| desktop_code_workspace_path(&data_dir, &w)),
            SessionKind::CoworkAgent => None,
        };
        per_install.push((install, sessions, ws_path));
    }

    // Whole-workspace share state (code panel only), computed once over the
    // per-install workspace paths via the bidirectional detector. Installs with
    // no workspace get a non-existent sentinel so they read independent/absent.
    let ws_paths: Vec<PathBuf> = per_install
        .iter()
        .enumerate()
        .map(|(i, (inst, _, ws))| {
            ws.clone()
                .unwrap_or_else(|| PathBuf::from(&inst.data_dir).join(format!("__no_ws_{i}")))
        })
        .collect();
    let ws_present: Vec<bool> = per_install
        .iter()
        .map(|(_, _, ws)| {
            ws.as_ref()
                .map(|p| {
                    p.is_dir()
                        || fs::symlink_metadata(p)
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .collect();
    let ws_states = symlink_share_states(&ws_paths, &ws_present);
    // Per-cell share state for a present cell, given its column index.
    let cell_state = |i: usize, present: bool| -> String {
        if !present {
            "absent".to_string()
        } else if matches!(kind, SessionKind::CodePanel) && ws_present[i] {
            ws_states[i].to_string()
        } else {
            "independent".to_string()
        }
    };

    // Group by "project key" — for code-panel that's `cwd`, for cowork
    // agent that's `processName` (the VM name).
    fn project_key(s: &LocalSession, kind: SessionKind) -> Option<String> {
        match kind {
            SessionKind::CodePanel => s.cwd.clone(),
            SessionKind::CoworkAgent => s.process_name.clone().or_else(|| s.cwd.clone()),
        }
    }

    let mut all_keys: BTreeMap<String, ()> = BTreeMap::new();
    for (_, sessions, _) in &per_install {
        for s in sessions {
            if let Some(k) = project_key(s, kind) {
                all_keys.insert(k, ());
            }
        }
    }

    // Sort keys by most-recent activity across all profiles (descending).
    let mut keys: Vec<String> = all_keys.into_keys().collect();
    keys.sort_by_key(|k| {
        let mut latest = 0_i64;
        for (_, sessions, _) in &per_install {
            for s in sessions {
                if project_key(s, kind).as_deref() == Some(k.as_str()) {
                    latest = latest.max(s.last_activity_ms);
                }
            }
        }
        -latest // descending
    });

    let mut rows: Vec<LibraryRow> = Vec::with_capacity(keys.len() + 1);

    // Synthetic top row — workspace-level summary, lets the user toggle
    // the symlink share for the *whole* workspace at once.
    if matches!(kind, SessionKind::CodePanel) {
        let cells: Vec<LibraryCell> = per_install
            .iter()
            .enumerate()
            .map(|(i, (install, sessions, ws_path))| {
                let n = sessions.len();
                let last = sessions.iter().map(|s| s.last_activity_ms).max().unwrap_or(0);
                let detail = if n > 0 {
                    Some(format!(
                        "{n} session{} · {}",
                        if n == 1 { "" } else { "s" },
                        if last > 0 { humanize_ago(last) } else { "—".into() }
                    ))
                } else {
                    None
                };
                LibraryCell {
                    install_id: install.id.clone(),
                    install_name: install.name.clone(),
                    data_dir: install.data_dir.clone(),
                    kind: install.kind.clone(),
                    state: cell_state(i, n > 0),
                    present: n > 0,
                    detail,
                    digest: None,
                    link_target_digest: ws_path.as_deref().and_then(symlink_target_digest),
                }
            })
            .collect();
        rows.push(LibraryRow {
            id: "__workspace__".into(),
            label: "Whole workspace".into(),
            description: Some(
                "Toggle to symlink the entire `claude-code-sessions/` workspace between profiles.".into(),
            ),
            cells,
            interactive: true,
            group: Some("Workspace".into()),
        });
    }

    // One row per cwd / processName.
    for key in keys {
        let cells: Vec<LibraryCell> = per_install
            .iter()
            .enumerate()
            .map(|(i, (install, sessions, ws_path))| {
                let matching_owned: Vec<LocalSession> = sessions
                    .iter()
                    .filter(|s| project_key(s, kind).as_deref() == Some(key.as_str()))
                    .cloned()
                    .collect();
                let present = !matching_owned.is_empty();
                let mut cell = build_session_cell(
                    install,
                    &matching_owned,
                    ws_path.as_deref().and_then(symlink_target_digest),
                );
                cell.state = cell_state(i, present);
                cell
            })
            .collect();

        // Pick the most-recent session for this key across ALL profiles —
        // it carries the human-readable `title` we'll use to surface what
        // this row is actually *about*.
        let best_session = per_install
            .iter()
            .flat_map(|(_, sessions, _)| sessions.iter())
            .filter(|s| project_key(s, kind).as_deref() == Some(key.as_str()))
            .max_by_key(|s| s.last_activity_ms);
        let best_title = best_session
            .and_then(|s| s.title.clone())
            .filter(|t| !t.is_empty());

        let basename = Path::new(&key)
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| key.clone());

        // Cowork spawns git worktrees under `<repo>/.claude/worktrees/<random>`;
        // basename is meaningless. Same story for Cowork agent VMs — their
        // processName is a "happy-rubin-dewdney" style placeholder. In both
        // cases, prefer the session title; fall back to basename only when
        // a session has no title yet (rare — Claude auto-titles on first
        // message).
        let is_random_dir = key.contains("/.claude/worktrees/")
            || matches!(kind, SessionKind::CoworkAgent);

        let label = if is_random_dir {
            best_title.clone().unwrap_or_else(|| basename.clone())
        } else {
            basename.clone()
        };

        // Description: tildified path for code panel, the VM name when it's
        // a Cowork agent run. If we used the title for the label *and* the
        // basename differs (worktree case), include both so the user can
        // still see "which clone".
        let home = std::env::var("HOME").unwrap_or_default();
        let description = match kind {
            SessionKind::CodePanel => {
                let path = key.replace(&home, "~");
                if is_random_dir && label != basename {
                    Some(format!("{path} · {basename}"))
                } else {
                    Some(path)
                }
            }
            SessionKind::CoworkAgent => {
                if label != key {
                    Some(format!("Cowork VM · {key}"))
                } else {
                    Some("Cowork VM".into())
                }
            }
        };

        // Per-cwd / per-process rows go under a content-aware bucket:
        // Cowork agent → "Cowork agent runs"; Cowork-spawned worktree dirs
        // → "Cowork worktrees"; real project cwds → "Projects".
        let group_label = match kind {
            SessionKind::CoworkAgent => "Cowork agent runs",
            SessionKind::CodePanel if key.contains("/.claude/worktrees/") => "Cowork worktrees",
            SessionKind::CodePanel => "Projects",
        };
        rows.push(LibraryRow {
            id: key,
            label,
            description,
            cells,
            // Per-cwd / per-process rows are browse-only — toggling them
            // wouldn't share just *one* cwd's sessions, because sessions
            // live as a workspace-bundled symlink. The user uses the
            // synthetic "Whole workspace" row to share, and clicks per-cwd
            // rows to inspect individual sessions in the DetailSheet.
            interactive: false,
            group: Some(group_label.to_string()),
        });
    }

    // Sort so rows in the same group are contiguous. Stable, so within-group
    // ordering (activity-descending from above) is preserved.
    rows.sort_by_key(|r| match r.group.as_deref() {
        Some("Workspace") => 0,
        Some("Projects") => 1,
        Some("Cowork worktrees") => 2,
        Some("Cowork agent runs") => 3,
        _ => 9,
    });

    Ok(rows)
}

/// Read individual sessions matching a project key from a given install.
/// Used by the DetailSheet to enumerate "what conversations happened here?"
pub fn list_sessions_for_project(
    install_id: String,
    row_id: String,
    is_cowork: bool,
) -> Result<Vec<LocalSession>, String> {
    let installs = list_desktop_installs()?;
    let install = installs
        .iter()
        .find(|i| i.id == install_id)
        .ok_or_else(|| format!("Profile {install_id} not found"))?;
    let data_dir = PathBuf::from(&install.data_dir);
    let root = if is_cowork {
        cowork_sessions_root(&data_dir)
    } else {
        code_sessions_root(&data_dir)
    };
    let sessions = scan_sessions_under(&root);
    let mut filtered: Vec<LocalSession> = sessions
        .into_iter()
        .filter(|s| {
            if row_id == "__workspace__" {
                true
            } else if is_cowork {
                s.process_name.as_deref() == Some(row_id.as_str())
                    || s.cwd.as_deref() == Some(row_id.as_str())
            } else {
                s.cwd.as_deref() == Some(row_id.as_str())
            }
        })
        .collect();
    filtered.sort_by_key(|s| -s.last_activity_ms);
    Ok(filtered)
}

/// "5m ago", "2h ago", "3d ago" — concise relative time for densely-packed
/// cells. Uses chrono::Utc::now() so it's testable and consistent with the
/// rest of the codebase.
fn humanize_ago(ms: i64) -> String {
    let now = Utc::now().timestamp_millis();
    let delta = (now - ms).max(0);
    let s = delta / 1000;
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else if s < 86_400 {
        format!("{}h ago", s / 3600)
    } else if s < 86_400 * 30 {
        format!("{}d ago", s / 86_400)
    } else {
        format!("{}mo ago", s / (86_400 * 30))
    }
}

// ----- Profile detail (codexbar-style stat panel) -----

/// One identity (Anthropic account) seen in this profile. A profile can
/// host the *owner* (whoever's logged in to Claude Desktop) plus zero or
/// more *co-users* who used Cowork agent mode under their own login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileIdentity {
    pub account_id: String,
    /// True iff this matches `cowork-enabled-cli-ops.json`'s ownerAccountId.
    pub is_owner: bool,
    /// Display name surfaced from any agent-mode session for this account.
    pub account_name: Option<String>,
    /// Email — same source.
    pub email_address: Option<String>,
    /// Cowork agent-mode sessions belonging to this account in this profile.
    pub agent_session_count: u32,
    /// Most-recent activity timestamp across this identity's sessions.
    pub last_activity_ms: Option<i64>,
}

// `f32` doesn't implement Eq (NaN), so this struct can only be PartialEq.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileStats {
    pub install_id: String,
    pub install_name: String,
    pub kind: String,
    pub data_dir: String,
    pub account_id: Option<String>,
    pub org_id: Option<String>,
    /// All identities (accounts) that have left a footprint in this profile.
    /// The owner appears first; co-users follow sorted by recency.
    pub identities: Vec<ProfileIdentity>,
    /// Tokens consumed today across all accounts in this Desktop instance,
    /// from `buddy-tokens.json`. Reset daily by Claude Desktop.
    pub tokens_today: u64,
    /// "YYYY-MM-DD" the tokens_today count is for. If stale (not today),
    /// the count is from a previous day and Desktop hasn't reset yet.
    pub tokens_today_date: Option<String>,
    /// codexbar-style time-window session counts. Computed from session
    /// files' lastActivityAt — gives the user the "do I have headroom?"
    /// reading without needing an Anthropic-side quota response.
    pub code_sessions_last_5h: u32,
    pub code_sessions_last_24h: u32,
    pub code_sessions_last_7d: u32,
    pub code_sessions_last_30d: u32,
    /// Avg sessions/day over the last 7 days (excluding today) — the
    /// "pace baseline" the UI compares today against.
    pub code_sessions_per_day_baseline: f32,
    /// Sessions started today. Same number drives the Today bar.
    pub code_sessions_today: u32,
    /// Most-used model across all code sessions in last 7d, normalized
    /// (e.g. "opus-4-7"). Top model only.
    pub top_model_last_7d: Option<String>,
    /// Device identifier from `ant-did` (base64-decoded UUID). Useful to
    /// spot when two profiles think they're on different machines.
    pub device_id: Option<String>,
    /// SSH remote configs (from `ssh_configs.json` → `configs`).
    pub ssh_remote_count: u32,
    /// Bytes-on-disk of the data directory. Computed via `du -sk` (fast).
    pub disk_bytes: Option<u64>,
    /// Sub-totals so the user sees where the Big GBs are.
    pub code_panel_bytes: Option<u64>,
    pub cowork_agent_bytes: Option<u64>,
    /// Unix millis — when the data dir was first created (mtime of the dir).
    pub created_at_ms: Option<i64>,
    /// Unix millis — last write activity anywhere in the dir.
    pub last_activity_ms: Option<i64>,
    /// Cowork code session count (from `claude-code-sessions/`).
    pub code_session_count: u32,
    pub code_total_bytes: u64,
    pub code_recent_cwds: Vec<String>,
    /// Cowork agent-mode session count (from `local-agent-mode-sessions/`).
    pub cowork_session_count: u32,
    /// Number of installed Desktop extensions.
    pub extension_count: u32,
    /// Number of MCP servers in claude_desktop_config.json.
    pub mcp_server_count: u32,
    /// Number of Cowork skills active in this profile's combo dir.
    pub cowork_skill_count: u32,
    /// 8-hex prefix of the link_target_digest of the workspace symlink,
    /// useful for "shared with these other profiles" badges.
    pub link_group: Option<String>,
    /// install_ids of other profiles that share this workspace.
    pub shared_with: Vec<String>,
}

/// Aggregate time-windowed session counts and model usage. Computed from
/// the same session files the matrix view scans.
struct CodeUsageWindows {
    last_5h: u32,
    last_24h: u32,
    last_7d: u32,
    last_30d: u32,
    today: u32,
    /// Avg sessions per day over the previous 7 days, excluding today.
    /// Returns 0.0 when there's not enough history.
    per_day_baseline: f32,
    top_model: Option<String>,
}

fn compute_code_usage_windows(sessions: &[LocalSession]) -> CodeUsageWindows {
    let now = Utc::now().timestamp_millis();
    let one_hour: i64 = 3_600_000;
    let one_day: i64 = 86_400_000;
    let mut last_5h = 0;
    let mut last_24h = 0;
    let mut last_7d = 0;
    let mut last_30d = 0;
    // Sessions per day for the last 8 days, indexed 0..=7 where 0 = today.
    let mut per_day = [0_u32; 8];
    // Today in epoch days (UTC start of day).
    let today_epoch_day = now / one_day;

    let mut model_counts: HashMap<String, u32> = HashMap::new();

    for s in sessions {
        let t = s.last_activity_ms;
        if t == 0 {
            continue;
        }
        let dt = now - t;
        if dt < 5 * one_hour {
            last_5h += 1;
        }
        if dt < one_day {
            last_24h += 1;
        }
        if dt < 7 * one_day {
            last_7d += 1;
            if let Some(m) = &s.model {
                let normalized = normalize_model_id(m);
                *model_counts.entry(normalized).or_insert(0) += 1;
            }
        }
        if dt < 30 * one_day {
            last_30d += 1;
        }
        let session_epoch_day = t / one_day;
        let days_ago = (today_epoch_day - session_epoch_day) as i64;
        if (0..8).contains(&days_ago) {
            per_day[days_ago as usize] += 1;
        }
    }

    let today_count = per_day[0];
    let prior_7_sum: u32 = per_day[1..=7].iter().sum();
    let per_day_baseline = (prior_7_sum as f32) / 7.0;

    let top_model = model_counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(m, _)| m);

    CodeUsageWindows {
        last_5h,
        last_24h,
        last_7d,
        last_30d,
        today: today_count,
        per_day_baseline,
        top_model,
    }
}

/// Drop `[1m]` / `[200k]` / etc. suffixes; lowercase the rest. e.g.
/// "claude-opus-4-7[1m]" → "opus-4-7".
fn normalize_model_id(raw: &str) -> String {
    let cleaned: String = raw
        .split('[')
        .next()
        .unwrap_or(raw)
        .trim()
        .to_string();
    cleaned
        .strip_prefix("claude-")
        .map(|s| s.to_string())
        .unwrap_or(cleaned)
        .to_lowercase()
}

/// Read tokens-today count from buddy-tokens.json. Missing file → (0, None).
fn read_tokens_today(data_dir: &Path) -> (u64, Option<String>) {
    let path = data_dir.join("buddy-tokens.json");
    let Ok(raw) = fs::read_to_string(&path) else { return (0, None) };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return (0, None);
    };
    let today = v.get("tokens-today");
    let count = today
        .and_then(|t| t.get("tokens"))
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let date = today
        .and_then(|t| t.get("date"))
        .and_then(|s| s.as_str())
        .map(String::from);
    (count, date)
}

/// Decode the base64 device id from `ant-did`. The file contains a single
/// base64-encoded UUID string. Missing/invalid → None.
fn read_device_id(data_dir: &Path) -> Option<String> {
    use base64::Engine;
    let raw = fs::read_to_string(data_dir.join("ant-did")).ok()?;
    let trimmed = raw.trim();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .ok()?;
    let s = String::from_utf8(bytes).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn read_ssh_remote_count(data_dir: &Path) -> u32 {
    let path = data_dir.join("ssh_configs.json");
    let Ok(raw) = fs::read_to_string(&path) else { return 0 };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return 0;
    };
    v.get("configs")
        .and_then(|c| c.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0)
}

/// Walk `local-agent-mode-sessions/` and group sessions by their owning
/// accountId (the directory two levels above each session JSON). Each
/// subdir at depth 1 *is* an accountId; the dir at depth 2 is per-org.
fn scan_identities(data_dir: &Path, owner_account: Option<&str>) -> Vec<ProfileIdentity> {
    let root = cowork_sessions_root(data_dir);
    let mut by_account: BTreeMap<String, (Vec<LocalSession>, ())> = BTreeMap::new();
    if let Ok(outer) = fs::read_dir(&root) {
        for entry in outer.flatten() {
            let outer_path = entry.path();
            if !outer_path.is_dir() {
                continue;
            }
            let acct = match outer_path.file_name().and_then(|n| n.to_str()) {
                Some("skills-plugin") | None => continue,
                Some(s) => s.to_string(),
            };
            // Scan deeper for sessions belonging to this account.
            let mut sessions: Vec<LocalSession> = Vec::new();
            if let Ok(inner) = fs::read_dir(&outer_path) {
                for inner_e in inner.flatten() {
                    let p = inner_e.path();
                    if !p.is_dir() {
                        continue;
                    }
                    if let Ok(leaf) = fs::read_dir(&p) {
                        for f in leaf.flatten() {
                            let fp = f.path();
                            if fp.file_name()
                                .and_then(|n| n.to_str())
                                .map(|s| s.starts_with("local_") && s.ends_with(".json"))
                                .unwrap_or(false)
                            {
                                if let Some(s) = parse_local_session(&fp) {
                                    sessions.push(s);
                                }
                            }
                        }
                    }
                }
            }
            by_account
                .entry(acct)
                .or_insert((Vec::new(), ()))
                .0
                .extend(sessions);
        }
    }

    // Build ProfileIdentity entries. Owner first, then by recency.
    let mut identities: Vec<ProfileIdentity> = by_account
        .into_iter()
        .map(|(account_id, (sessions, _))| {
            let is_owner = owner_account == Some(account_id.as_str());
            let mut sorted = sessions;
            sorted.sort_by_key(|s| -s.last_activity_ms);
            let latest = sorted.first();
            ProfileIdentity {
                is_owner,
                account_name: latest.and_then(|s| s.account_name.clone()),
                email_address: latest.and_then(|s| s.email_address.clone()),
                last_activity_ms: latest.map(|s| s.last_activity_ms),
                agent_session_count: sorted.len() as u32,
                account_id,
            }
        })
        .collect();

    // If the owner has no agent-mode sessions, still surface them as an
    // identity with empty fields — they're the profile's primary account
    // and the UI needs to render them somewhere.
    if let Some(owner) = owner_account {
        if !identities.iter().any(|i| i.account_id == owner) {
            identities.push(ProfileIdentity {
                account_id: owner.to_string(),
                is_owner: true,
                account_name: None,
                email_address: None,
                agent_session_count: 0,
                last_activity_ms: None,
            });
        }
    }

    identities.sort_by(|a, b| {
        b.is_owner
            .cmp(&a.is_owner)
            .then_with(|| b.last_activity_ms.cmp(&a.last_activity_ms))
    });
    identities
}

fn dir_disk_bytes(path: &Path) -> Option<u64> {
    // `du -sk` is heavily optimized for this and beats a naive walkdir for
    // huge dirs (Claude data dirs routinely hit 5-15 GB).
    let out = Command::new("/usr/bin/du")
        .args(["-sk", "-x"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let kb: u64 = s.split_whitespace().next()?.parse().ok()?;
    Some(kb * 1024)
}

fn dir_created_ms(path: &Path) -> Option<i64> {
    let meta = fs::metadata(path).ok()?;
    // On macOS, `created()` returns birth time. Fall back to modified.
    let t = meta.created().ok().or_else(|| meta.modified().ok())?;
    Some(system_time_to_epoch_ms(t))
}

pub fn get_profile_stats(install_id: String) -> Result<ProfileStats, String> {
    let installs = list_desktop_installs()?;
    let install = installs
        .iter()
        .find(|i| i.id == install_id)
        .ok_or_else(|| format!("Profile {install_id} not found"))?
        .clone();

    let data_dir = PathBuf::from(&install.data_dir);
    let account_id = read_account_id(&data_dir).unwrap_or(None);
    let org_id = read_org_id(&data_dir).unwrap_or(None);

    let stat = scan_desktop_code_history_with_data_dir(
        &data_dir,
        &desktop_code_sessions_path(&data_dir),
    )
    .unwrap_or_default();

    // Per-identity breakdown: owner from cowork-enabled-cli-ops.json,
    // co-users from any other accountId subdir present on disk. Each is
    // tagged with name/email pulled from their most-recent session file.
    let identities = scan_identities(&data_dir, account_id.as_deref());

    // Aggregate counts across all identities for the "total Cowork sessions"
    // headline.
    let cowork_sessions_total: u32 = identities.iter().map(|i| i.agent_session_count).sum();

    let extensions = list_extensions_in_dir(&data_dir).unwrap_or_default();
    let config = read_desktop_config(&data_dir).unwrap_or(serde_json::json!({}));
    let mcp_count = mcp_servers_obj(&config).map(|m| m.len()).unwrap_or(0);
    let cowork_skills = find_skills_combo_dir(&data_dir)
        .unwrap_or(None)
        .and_then(|combo| read_skills_manifest(&combo).ok())
        .map(|m| manifest_skill_entries(&m).len())
        .unwrap_or(0);

    // Link group: digest of the workspace symlink target. Then find any
    // other profile that points to the same canonical target.
    let primary_path = stat
        .primary_workspace
        .as_ref()
        .map(|ws| desktop_code_workspace_path(&data_dir, ws));
    let link_digest = primary_path.as_deref().and_then(symlink_target_digest);
    let mut shared_with: Vec<String> = Vec::new();
    if let Some(ref my_digest) = link_digest {
        for other in &installs {
            if other.id == install.id {
                continue;
            }
            let od = PathBuf::from(&other.data_dir);
            let ostat = scan_desktop_code_history_with_data_dir(
                &od,
                &desktop_code_sessions_path(&od),
            )
            .unwrap_or_default();
            let opath = ostat
                .primary_workspace
                .as_ref()
                .map(|ws| desktop_code_workspace_path(&od, ws));
            if let Some(od_path) = opath {
                if let Some(d) = symlink_target_digest(&od_path) {
                    if &d == my_digest {
                        shared_with.push(other.id.clone());
                    }
                }
            }
        }
    }

    let (tokens_today, tokens_today_date) = read_tokens_today(&data_dir);
    let device_id = read_device_id(&data_dir);
    let ssh_remote_count = read_ssh_remote_count(&data_dir);
    let cowork_agent_bytes = dir_disk_bytes(&cowork_sessions_root(&data_dir));

    // Time-windowed counts from actual code panel session files (richer
    // than the aggregate stat which only carries totals).
    let code_sessions_all = scan_sessions_under(&code_sessions_root(&data_dir));
    let windows = compute_code_usage_windows(&code_sessions_all);

    Ok(ProfileStats {
        install_id: install.id,
        install_name: install.name,
        kind: install.kind,
        data_dir: install.data_dir.clone(),
        account_id,
        org_id,
        identities,
        tokens_today,
        tokens_today_date,
        code_sessions_last_5h: windows.last_5h,
        code_sessions_last_24h: windows.last_24h,
        code_sessions_last_7d: windows.last_7d,
        code_sessions_last_30d: windows.last_30d,
        code_sessions_per_day_baseline: windows.per_day_baseline,
        code_sessions_today: windows.today,
        top_model_last_7d: windows.top_model,
        device_id,
        ssh_remote_count,
        disk_bytes: dir_disk_bytes(&data_dir),
        code_panel_bytes: dir_disk_bytes(&desktop_code_sessions_path(&data_dir)),
        cowork_agent_bytes,
        created_at_ms: dir_created_ms(&data_dir),
        last_activity_ms: if stat.last_activity_ms > 0 {
            Some(stat.last_activity_ms)
        } else {
            None
        },
        code_session_count: stat.session_count,
        code_total_bytes: stat.total_bytes,
        code_recent_cwds: stat.recent_cwds,
        cowork_session_count: cowork_sessions_total,
        extension_count: extensions.len() as u32,
        mcp_server_count: mcp_count as u32,
        cowork_skill_count: cowork_skills as u32,
        link_group: link_digest.map(|d| d.chars().take(8).collect()),
        shared_with,
    })
}

mod commands {
    use super::*;

    #[tauri::command]
    pub fn list_desktop_installs() -> Result<Vec<DesktopInstall>, String> {
        super::list_desktop_installs()
    }

    #[tauri::command]
    pub fn create_desktop_profile(name: String) -> Result<DesktopInstall, String> {
        super::create_desktop_profile(name)
    }

    #[tauri::command]
    pub fn create_code_profile(
        name: String,
        seed_from_default: bool,
    ) -> Result<CodeInstall, String> {
        super::create_code_profile(name, seed_from_default)
    }

    #[tauri::command]
    pub fn launch_desktop_install(install_id: String) -> Result<(), String> {
        super::launch_desktop_install(install_id)
    }

    #[tauri::command]
    pub fn list_extension_matrix(
        source_data_dir: String,
        target_data_dir: String,
    ) -> Result<Vec<ExtensionSelectionRow>, String> {
        super::list_extension_matrix(source_data_dir, target_data_dir)
    }

    #[tauri::command]
    pub fn copy_selected_extensions(
        source_data_dir: String,
        target_data_dir: String,
        extension_ids: Vec<String>,
    ) -> Result<CopySummary, String> {
        super::copy_selected_extensions(source_data_dir, target_data_dir, extension_ids)
    }

    #[tauri::command]
    pub fn list_extension_library() -> Result<Vec<ExtensionShareItem>, String> {
        super::list_extension_library()
    }

    #[tauri::command]
    pub fn copy_extension_to_targets(
        source_data_dir: String,
        target_data_dirs: Vec<String>,
        extension_id: String,
    ) -> Result<CopySummary, String> {
        super::copy_extension_to_targets(source_data_dir, target_data_dirs, extension_id)
    }

    #[tauri::command]
    pub fn list_pair_sharing(
        source_data_dir: String,
        target_data_dir: String,
    ) -> Result<Vec<PairExtensionShare>, String> {
        super::list_pair_sharing(source_data_dir, target_data_dir)
    }

    #[tauri::command]
    pub fn apply_pair_sharing(
        source_data_dir: String,
        target_data_dir: String,
        changes: Vec<PairShareChange>,
    ) -> Result<CopySummary, String> {
        super::apply_pair_sharing(source_data_dir, target_data_dir, changes)
    }

    #[tauri::command]
    pub fn list_code_installs() -> Result<Vec<CodeInstall>, String> {
        super::list_code_installs()
    }

    #[tauri::command]
    pub fn list_codex_installs() -> Result<Vec<CodexInstall>, String> {
        super::list_codex_installs()
    }

    #[tauri::command]
    pub fn create_codex_profile(name: String) -> Result<CodexInstall, String> {
        super::create_codex_profile(name)
    }

    #[tauri::command]
    pub fn launch_codex_install(install_id: String) -> Result<(), String> {
        super::launch_codex_install(install_id)
    }

    #[tauri::command]
    pub fn delete_desktop_profile(install_id: String, delete_data: bool) -> Result<(), String> {
        super::delete_desktop_profile(install_id, delete_data)
    }

    #[tauri::command]
    pub fn delete_codex_profile(install_id: String, delete_data: bool) -> Result<(), String> {
        super::delete_codex_profile(install_id, delete_data)
    }

    #[tauri::command]
    pub fn import_codex_session_to_claude(
        source: String,
    ) -> Result<super::convert::ImportResult, String> {
        super::import_codex_session_to_claude_any_home(source)
    }

    #[tauri::command]
    pub fn import_claude_session_to_codex(
        source: String,
    ) -> Result<super::convert::ImportResult, String> {
        super::convert::import_claude_session_to_codex(source)
    }

    #[tauri::command]
    pub fn import_codex_project_to_claude(
        install_id: String,
        cwd: String,
    ) -> Result<super::BatchImportResult, String> {
        super::import_codex_project_to_claude(install_id, cwd)
    }

    #[tauri::command]
    pub fn import_claude_project_to_codex(
        install_id: String,
        project_id: String,
    ) -> Result<super::BatchImportResult, String> {
        super::import_claude_project_to_codex(install_id, project_id)
    }

    #[tauri::command]
    pub fn import_all_codex_to_claude(install_id: String) -> Result<super::BatchImportResult, String> {
        super::import_all_codex_to_claude(install_id)
    }

    #[tauri::command]
    pub fn import_all_claude_to_codex(install_id: String) -> Result<super::BatchImportResult, String> {
        super::import_all_claude_to_codex(install_id)
    }

    #[tauri::command]
    pub fn list_code_history(config_dir: String) -> Result<Vec<CodeProject>, String> {
        super::list_code_history(Path::new(&config_dir))
    }

    #[tauri::command]
    pub fn list_pair_code_history_sharing(
        source_config_dir: String,
        target_config_dir: String,
    ) -> Result<Vec<PairCodeProjectShare>, String> {
        super::list_pair_code_history_sharing(source_config_dir, target_config_dir)
    }

    #[tauri::command]
    pub fn apply_pair_code_history_sharing(
        source_config_dir: String,
        target_config_dir: String,
        changes: Vec<PairCodeShareChange>,
    ) -> Result<CopySummary, String> {
        super::apply_pair_code_history_sharing(source_config_dir, target_config_dir, changes)
    }

    #[tauri::command]
    pub fn list_pair_desktop_code_history(
        source_data_dir: String,
        target_data_dir: String,
    ) -> Result<PairDesktopCodeHistory, String> {
        super::list_pair_desktop_code_history(source_data_dir, target_data_dir)
    }

    #[tauri::command]
    pub fn apply_pair_desktop_code_history(
        source_data_dir: String,
        target_data_dir: String,
        change: PairDesktopCodeHistoryChange,
    ) -> Result<CopySummary, String> {
        super::apply_pair_desktop_code_history(source_data_dir, target_data_dir, change)
    }

    #[tauri::command]
    pub fn list_pair_mcp_sharing(
        source_data_dir: String,
        target_data_dir: String,
    ) -> Result<Vec<PairMcpServerShare>, String> {
        super::list_pair_mcp_sharing(source_data_dir, target_data_dir)
    }

    #[tauri::command]
    pub fn apply_pair_mcp_sharing(
        source_data_dir: String,
        target_data_dir: String,
        changes: Vec<PairMcpServerChange>,
    ) -> Result<CopySummary, String> {
        super::apply_pair_mcp_sharing(source_data_dir, target_data_dir, changes)
    }

    #[tauri::command]
    pub fn list_pair_cowork_skills_sharing(
        source_data_dir: String,
        target_data_dir: String,
    ) -> Result<PairCoworkSkillsResult, String> {
        super::list_pair_cowork_skills_sharing(source_data_dir, target_data_dir)
    }

    #[tauri::command]
    pub fn apply_pair_cowork_skills_sharing(
        source_data_dir: String,
        target_data_dir: String,
        changes: Vec<PairCoworkSkillChange>,
    ) -> Result<CopySummary, String> {
        super::apply_pair_cowork_skills_sharing(source_data_dir, target_data_dir, changes)
    }

    #[tauri::command]
    pub fn list_pair_preference_sharing(
        source_data_dir: String,
        target_data_dir: String,
    ) -> Result<Vec<PairPreferenceShare>, String> {
        super::list_pair_preference_sharing(source_data_dir, target_data_dir)
    }

    #[tauri::command]
    pub fn apply_pair_preference_sharing(
        source_data_dir: String,
        target_data_dir: String,
        changes: Vec<PairPreferenceChange>,
    ) -> Result<CopySummary, String> {
        super::apply_pair_preference_sharing(source_data_dir, target_data_dir, changes)
    }

    // Library / matrix view — one call returns a row × profile grid for a kind.
    #[tauri::command]
    pub fn list_library_extensions() -> Result<Vec<LibraryRow>, String> {
        super::list_extensions_library_grid()
    }

    #[tauri::command]
    pub fn list_library_mcp() -> Result<Vec<LibraryRow>, String> {
        super::list_mcp_library()
    }

    #[tauri::command]
    pub fn list_library_cowork_skills() -> Result<Vec<LibraryRow>, String> {
        super::list_cowork_skills_library()
    }

    #[tauri::command]
    pub fn list_skills_library() -> Result<Vec<LibraryRow>, String> {
        super::list_skills_library()
    }

    #[tauri::command]
    pub fn list_codex_sessions_library() -> Result<Vec<LibraryRow>, String> {
        super::list_codex_sessions_library()
    }

    #[tauri::command]
    pub fn list_codex_skills_library() -> Result<Vec<LibraryRow>, String> {
        super::list_codex_skills_library()
    }

    #[tauri::command]
    pub fn list_codex_mcp_library() -> Result<Vec<LibraryRow>, String> {
        super::list_codex_mcp_library()
    }

    #[tauri::command]
    pub fn list_codex_sessions_for_project(
        install_id: String,
        cwd: String,
    ) -> Result<Vec<LocalSession>, String> {
        super::list_codex_sessions_for_project(install_id, cwd)
    }

    #[tauri::command]
    pub fn list_claude_sessions_library() -> Result<Vec<LibraryRow>, String> {
        super::list_claude_sessions_library()
    }

    #[tauri::command]
    pub fn list_claude_sessions_for_project(
        install_id: String,
        project_id: String,
    ) -> Result<Vec<LocalSession>, String> {
        super::list_claude_sessions_for_project(install_id, project_id)
    }

    #[tauri::command]
    pub fn list_claude_skills_library() -> Result<Vec<LibraryRow>, String> {
        super::list_claude_skills_library()
    }

    #[tauri::command]
    pub fn list_codex_preferences_library() -> Result<Vec<LibraryRow>, String> {
        super::list_codex_preferences_library()
    }

    #[tauri::command]
    pub fn read_text_file(path: String) -> Result<String, String> {
        super::read_text_file(path)
    }

    #[tauri::command]
    pub fn write_text_file(path: String, content: String) -> Result<(), String> {
        super::write_text_file(path, content)
    }

    #[tauri::command]
    pub fn delete_content_path(path: String) -> Result<(), String> {
        super::delete_content_path(path)
    }

    #[tauri::command]
    pub fn import_memory_file(source: String, target: String) -> Result<(), String> {
        super::import_memory_file(source, target)
    }

    #[tauri::command]
    pub fn read_mcp_server(config_path: String, server: String) -> Result<String, String> {
        super::read_mcp_server(config_path, server)
    }

    #[tauri::command]
    pub fn write_mcp_server(
        config_path: String,
        server: String,
        body: String,
    ) -> Result<(), String> {
        super::write_mcp_server(config_path, server, body)
    }

    #[tauri::command]
    pub fn delete_mcp_server(config_path: String, server: String) -> Result<(), String> {
        super::delete_mcp_server(config_path, server)
    }

    #[tauri::command]
    pub fn get_session_transcript(
        install_id: String,
        session_id: String,
        world: String,
    ) -> Result<String, String> {
        super::get_session_transcript(install_id, session_id, world)
    }

    #[tauri::command]
    pub fn delete_session_file(
        install_id: String,
        session_id: String,
        world: String,
    ) -> Result<(), String> {
        super::delete_session_file(install_id, session_id, world)
    }

    #[tauri::command]
    pub fn list_mcp_cross_library() -> Result<Vec<LibraryRow>, String> {
        super::list_mcp_cross_library()
    }

    #[tauri::command]
    pub fn list_memory_library() -> Result<Vec<LibraryRow>, String> {
        super::list_memory_library()
    }

    #[tauri::command]
    pub fn list_claude_memory_library() -> Result<Vec<LibraryRow>, String> {
        super::list_claude_memory_library()
    }

    #[tauri::command]
    pub fn list_codex_memory_library() -> Result<Vec<LibraryRow>, String> {
        super::list_codex_memory_library()
    }

    #[tauri::command]
    pub fn list_library_preferences() -> Result<Vec<LibraryRow>, String> {
        super::list_preferences_library()
    }

    #[tauri::command]
    pub fn apply_library_changes(
        kind: String,
        changes: Vec<LibraryCellChange>,
    ) -> Result<CopySummary, String> {
        super::apply_library_changes(kind, changes)
    }

    #[tauri::command]
    pub fn list_library_code_history() -> Result<Vec<LibraryRow>, String> {
        super::list_code_history_library()
    }

    #[tauri::command]
    pub fn list_library_cowork_sessions() -> Result<Vec<LibraryRow>, String> {
        super::list_cowork_sessions_library()
    }

    #[tauri::command]
    pub fn list_sessions_for_project(
        install_id: String,
        row_id: String,
        is_cowork: bool,
    ) -> Result<Vec<LocalSession>, String> {
        super::list_sessions_for_project(install_id, row_id, is_cowork)
    }

    #[tauri::command]
    pub fn get_profile_stats(install_id: String) -> Result<ProfileStats, String> {
        super::get_profile_stats(install_id)
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_decorum::init())
        .setup(|app| {
            // The canonical Tauri 2 way to get a single-row overlay title
            // bar with traffic lights inset to match a custom toolbar.
            // tauri-plugin-decorum wraps the NSWindow calls + transparency
            // setup that would otherwise be hand-rolled per-app.
            //
            // Equivalent formula derived from Claude.app's compiled config:
            //   inset_y = (toolbar_height_px - light_height_px) / 2
            //           = (45 - 12) / 2 = 16.5 → 17
            // The plugin's set_traffic_lights_inset takes (x, y) in points;
            // we use the same value for both to get a symmetric inset.
            use tauri::Manager;
            use tauri_plugin_decorum::WebviewWindowExt;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.create_overlay_titlebar();
                #[cfg(target_os = "macos")]
                {
                    let _ = window.set_traffic_lights_inset(17.0, 17.0);
                    let _ = window.make_transparent();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Clicking the window close button (red ✕ / Cmd-W) just HIDES the
            // window — the app keeps running in the background. Clicking the dock
            // icon re-shows it (RunEvent::Reopen below). Cmd-Q / the app menu
            // still fully quit.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_desktop_installs,
            commands::create_desktop_profile,
            commands::launch_desktop_install,
            commands::list_extension_matrix,
            commands::copy_selected_extensions,
            commands::list_extension_library,
            commands::copy_extension_to_targets,
            commands::list_pair_sharing,
            commands::apply_pair_sharing,
            commands::create_code_profile,
            commands::list_code_installs,
            commands::list_codex_installs,
            commands::create_codex_profile,
            commands::launch_codex_install,
            commands::delete_desktop_profile,
            commands::delete_codex_profile,
            commands::import_codex_session_to_claude,
            commands::import_claude_session_to_codex,
            commands::import_codex_project_to_claude,
            commands::import_claude_project_to_codex,
            commands::import_all_codex_to_claude,
            commands::import_all_claude_to_codex,
            commands::list_code_history,
            commands::list_pair_code_history_sharing,
            commands::apply_pair_code_history_sharing,
            commands::list_pair_desktop_code_history,
            commands::apply_pair_desktop_code_history,
            commands::list_pair_mcp_sharing,
            commands::apply_pair_mcp_sharing,
            commands::list_pair_cowork_skills_sharing,
            commands::apply_pair_cowork_skills_sharing,
            commands::list_pair_preference_sharing,
            commands::apply_pair_preference_sharing,
            commands::list_library_extensions,
            commands::list_library_mcp,
            commands::list_library_cowork_skills,
            commands::list_skills_library,
            commands::list_codex_sessions_library,
            commands::list_codex_skills_library,
            commands::list_codex_mcp_library,
            commands::list_codex_sessions_for_project,
            commands::list_claude_sessions_library,
            commands::list_claude_sessions_for_project,
            commands::list_claude_skills_library,
            commands::list_codex_preferences_library,
            commands::read_text_file,
            commands::write_text_file,
            commands::delete_content_path,
            commands::import_memory_file,
            commands::read_mcp_server,
            commands::write_mcp_server,
            commands::delete_mcp_server,
            commands::get_session_transcript,
            commands::delete_session_file,
            commands::list_mcp_cross_library,
            commands::list_memory_library,
            commands::list_claude_memory_library,
            commands::list_codex_memory_library,
            commands::list_library_preferences,
            commands::apply_library_changes,
            commands::list_library_code_history,
            commands::list_library_cowork_sessions,
            commands::list_sessions_for_project,
            commands::get_profile_stats
        ])
        .build(tauri::generate_context!())
        .expect("error while building Claudex")
        .run(|app, event| {
            // macOS: clicking the dock icon while the window is hidden re-shows it.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = &event {
                use tauri::Manager;
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Serializes tests that read or mutate the process-global HOME env, so the
    /// e2e share test (which points HOME at a tempdir) can't race the
    /// home_dir()-reading tests under parallel `cargo test`.
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn sanitize_profile_name_matches_cli_rules() {
        assert_eq!(sanitize_profile_name("  WORK  "), "work");
        assert_eq!(sanitize_profile_name("Client ACME"), "client-acme");
        assert_eq!(sanitize_profile_name("foo!!!bar"), "foo-bar");
        assert_eq!(sanitize_profile_name("--leading--"), "leading");
        assert_eq!(sanitize_profile_name("multi   spaces"), "multi-spaces");
    }

    #[test]
    fn registry_round_trips_existing_cli_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("profiles.json");
        let registry = RegistryFile {
            version: 1,
            profiles: vec![RegistryProfile {
                name: "work".to_string(),
                profile_type: "desktop".to_string(),
                desktop: Some(RegistryDesktop {
                    data_dir: "/Users/me/Library/Application Support/Claude-WORK".to_string(),
                    app_path: "/Users/me/Applications/Claude WORK.app".to_string(),
                    claude_app_path: "/Applications/Claude.app".to_string(),
                }),
                code: None,
                codex: None,
                created_at: "2026-05-22T12:00:00.000Z".to_string(),
            }],
        };

        save_registry_to_path(&path, &registry).unwrap();
        let loaded = load_registry_from_path(&path).unwrap();

        assert_eq!(loaded, registry);
        let raw = fs::read_to_string(path).unwrap();
        assert!(raw.contains("\"createdAt\""));
        assert!(raw.contains("\"dataDir\""));
    }

    #[test]
    fn list_extensions_reports_settings_presence_sorted_by_id() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("Claude Extensions");
        let settings_dir = tmp.path().join("Claude Extensions Settings");
        fs::create_dir_all(ext_dir.join("zeta")).unwrap();
        fs::create_dir_all(ext_dir.join("alpha")).unwrap();
        fs::create_dir_all(&settings_dir).unwrap();
        fs::write(settings_dir.join("alpha.json"), "{}").unwrap();

        let extensions = list_extensions_in_dir(tmp.path()).unwrap();

        assert_eq!(
            extensions,
            vec![
                ExtensionEntry {
                    id: "alpha".to_string(),
                    has_settings: true,
                },
                ExtensionEntry {
                    id: "zeta".to_string(),
                    has_settings: false,
                },
            ]
        );
    }

    #[test]
    fn copy_extension_replaces_folder_and_matching_settings_only() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        let source_ext = source.path().join("Claude Extensions").join("shared-one");
        let source_settings = source
            .path()
            .join("Claude Extensions Settings")
            .join("shared-one.json");
        fs::create_dir_all(&source_ext).unwrap();
        fs::create_dir_all(source_settings.parent().unwrap()).unwrap();
        fs::write(source_ext.join("manifest.json"), "{\"fresh\":true}").unwrap();
        fs::write(&source_settings, "{\"enabled\":true}").unwrap();

        let stale_target_ext = target.path().join("Claude Extensions").join("shared-one");
        fs::create_dir_all(&stale_target_ext).unwrap();
        fs::write(stale_target_ext.join("old.txt"), "stale").unwrap();

        copy_extension_between_dirs(source.path(), target.path(), "shared-one").unwrap();

        let copied_manifest = target
            .path()
            .join("Claude Extensions")
            .join("shared-one")
            .join("manifest.json");
        let copied_settings = target
            .path()
            .join("Claude Extensions Settings")
            .join("shared-one.json");

        assert_eq!(fs::read_to_string(copied_manifest).unwrap(), "{\"fresh\":true}");
        assert_eq!(fs::read_to_string(copied_settings).unwrap(), "{\"enabled\":true}");
        assert!(!stale_target_ext.join("old.txt").exists());
    }

    #[test]
    fn extension_library_is_content_first_and_includes_default_as_target() {
        let default_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        let client_dir = tempfile::tempdir().unwrap();

        fs::create_dir_all(default_dir.path().join("Claude Extensions").join("theme-kit")).unwrap();
        fs::create_dir_all(default_dir.path().join("Claude Extensions Settings")).unwrap();
        fs::write(
            default_dir
                .path()
                .join("Claude Extensions Settings")
                .join("theme-kit.json"),
            "{}",
        )
        .unwrap();
        fs::create_dir_all(work_dir.path().join("Claude Extensions").join("theme-kit")).unwrap();
        fs::create_dir_all(client_dir.path().join("Claude Extensions").join("mcp-helper")).unwrap();

        let installs = vec![
            DesktopInstall {
                id: "default".to_string(),
                name: "default".to_string(),
                kind: "default".to_string(),
                data_dir: default_dir.path().to_string_lossy().to_string(),
                app_path: None,
                launcher_path: None,
                managed: false,
                is_running: false,
            },
            DesktopInstall {
                id: "profile:work".to_string(),
                name: "work".to_string(),
                kind: "profile".to_string(),
                data_dir: work_dir.path().to_string_lossy().to_string(),
                app_path: None,
                launcher_path: None,
                managed: true,
                is_running: false,
            },
            DesktopInstall {
                id: "profile:client".to_string(),
                name: "client".to_string(),
                kind: "profile".to_string(),
                data_dir: client_dir.path().to_string_lossy().to_string(),
                app_path: None,
                launcher_path: None,
                managed: true,
                is_running: false,
            },
        ];

        let library = build_extension_library(&installs).unwrap();
        let theme = library.iter().find(|item| item.id == "theme-kit").unwrap();

        assert_eq!(
            library.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            vec!["mcp-helper", "theme-kit"]
        );
        assert_eq!(theme.sources.len(), 2);
        assert!(theme.targets.iter().any(|target| {
            target.install_id == "default" && target.has_extension && target.has_settings
        }));
        assert!(theme.targets.iter().any(|target| {
            target.install_id == "profile:client" && !target.has_extension
        }));
    }

    #[test]
    fn copy_extension_to_targets_applies_one_content_item_to_multiple_profiles() {
        let source = tempfile::tempdir().unwrap();
        let target_a = tempfile::tempdir().unwrap();
        let target_b = tempfile::tempdir().unwrap();

        let source_ext = source.path().join("Claude Extensions").join("shared-one");
        let source_settings = source
            .path()
            .join("Claude Extensions Settings")
            .join("shared-one.json");
        fs::create_dir_all(&source_ext).unwrap();
        fs::create_dir_all(source_settings.parent().unwrap()).unwrap();
        fs::write(source_ext.join("manifest.json"), "{\"fresh\":true}").unwrap();
        fs::write(&source_settings, "{\"enabled\":true}").unwrap();

        let summary = copy_extension_to_target_dirs(
            source.path(),
            &[target_a.path().to_path_buf(), target_b.path().to_path_buf()],
            "shared-one",
        )
        .unwrap();

        assert_eq!(summary.copied, 2);
        assert_eq!(summary.skipped, 0);
        assert!(target_a
            .path()
            .join("Claude Extensions")
            .join("shared-one")
            .join("manifest.json")
            .exists());
        assert!(target_b
            .path()
            .join("Claude Extensions Settings")
            .join("shared-one.json")
            .exists());
    }

    #[test]
    fn pair_share_state_detects_symlinked_extension_between_two_profiles() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let source_ext = source.path().join("Claude Extensions").join("shared-one");
        let source_settings = source
            .path()
            .join("Claude Extensions Settings")
            .join("shared-one.json");
        fs::create_dir_all(&source_ext).unwrap();
        fs::create_dir_all(source_settings.parent().unwrap()).unwrap();
        fs::write(source_ext.join("manifest.json"), "{}").unwrap();
        fs::write(&source_settings, "{}").unwrap();

        set_pair_extension_shared(source.path(), target.path(), "shared-one", true).unwrap();
        let rows = list_pair_extension_shares(source.path(), target.path()).unwrap();
        let row = rows.iter().find(|row| row.id == "shared-one").unwrap();

        assert!(row.shared);
        assert_eq!(row.direction, "source-to-target");
        assert!(target
            .path()
            .join("Claude Extensions")
            .join("shared-one")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn unchecking_pair_share_turns_symlink_back_into_independent_copy() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let source_ext = source.path().join("Claude Extensions").join("shared-one");
        fs::create_dir_all(&source_ext).unwrap();
        fs::write(source_ext.join("manifest.json"), "{\"fresh\":true}").unwrap();

        set_pair_extension_shared(source.path(), target.path(), "shared-one", true).unwrap();
        set_pair_extension_shared(source.path(), target.path(), "shared-one", false).unwrap();

        let target_ext = target.path().join("Claude Extensions").join("shared-one");
        assert!(target_ext.is_dir());
        assert!(!target_ext.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(
            fs::read_to_string(target_ext.join("manifest.json")).unwrap(),
            "{\"fresh\":true}"
        );
    }

    fn write_session_jsonl(dir: &Path, sid: &str, prompt: &str, extra_lines: usize) -> PathBuf {
        let path = dir.join(format!("{sid}.jsonl"));
        let header = serde_json::json!({
            "type": "queue-operation",
            "operation": "enqueue",
            "timestamp": "2026-04-22T07:14:05.312Z",
            "sessionId": sid,
            "content": prompt,
        })
        .to_string();
        let mut body = String::new();
        body.push_str(&header);
        body.push('\n');
        for i in 0..extra_lines {
            body.push_str(&format!("{{\"type\":\"noise\",\"i\":{i}}}\n"));
        }
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn list_code_history_reports_session_count_size_and_preview() {
        let cfg = tempfile::tempdir().unwrap();
        let projects = cfg.path().join(CODE_PROJECTS_DIR);
        fs::create_dir_all(&projects).unwrap();

        let proj_a = projects.join("-Users-foo-alpha");
        let proj_b = projects.join("-Users-foo-beta");
        fs::create_dir_all(&proj_a).unwrap();
        fs::create_dir_all(&proj_b).unwrap();
        write_session_jsonl(&proj_a, "11111111-1111-1111-1111-111111111111", "hello alpha", 0);
        write_session_jsonl(&proj_a, "22222222-2222-2222-2222-222222222222", "hello again", 0);
        write_session_jsonl(&proj_b, "33333333-3333-3333-3333-333333333333", "world beta", 0);

        // The lonely "-" placeholder dir Claude Code occasionally creates is ignored.
        fs::create_dir_all(projects.join("-")).unwrap();

        let projects_out = list_code_history(cfg.path()).unwrap();
        assert_eq!(projects_out.len(), 2);

        let alpha = projects_out.iter().find(|p| p.id == "-Users-foo-alpha").unwrap();
        assert_eq!(alpha.session_count, 2);
        assert!(alpha.total_bytes > 0);
        assert!(alpha.first_message_preview.is_some());
        assert_eq!(alpha.display_path, "/Users/foo/alpha");
    }

    #[test]
    fn pair_code_history_share_is_live_symlink_and_reversible() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let proj_id = "-Users-foo-shared";
        let source_proj = source.path().join(CODE_PROJECTS_DIR).join(proj_id);
        fs::create_dir_all(&source_proj).unwrap();
        write_session_jsonl(&source_proj, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "from source", 0);

        // Share A -> B.
        let summary = apply_pair_code_history_sharing(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
            vec![PairCodeShareChange { project_id: proj_id.to_string(), shared: true }],
        )
        .unwrap();
        assert_eq!(summary.copied, 1);

        let target_proj = target.path().join(CODE_PROJECTS_DIR).join(proj_id);
        assert!(target_proj.symlink_metadata().unwrap().file_type().is_symlink());

        // Live: appending a new session in source must show up under target.
        write_session_jsonl(&source_proj, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", "live append", 0);
        let pair_rows = list_pair_code_history_shares(source.path(), target.path()).unwrap();
        let row = pair_rows.iter().find(|r| r.id == proj_id).unwrap();
        assert!(row.shared);
        assert_eq!(row.source_session_count, 2);
        assert_eq!(row.target_session_count, 2);

        // Unshare: target becomes an independent copy of the current source state.
        let summary = apply_pair_code_history_sharing(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
            vec![PairCodeShareChange { project_id: proj_id.to_string(), shared: false }],
        )
        .unwrap();
        assert_eq!(summary.copied, 1);
        assert!(target_proj.is_dir());
        assert!(!target_proj.symlink_metadata().unwrap().file_type().is_symlink());

        // Now editing source should NOT touch target.
        write_session_jsonl(&source_proj, "cccccccc-cccc-cccc-cccc-cccccccccccc", "post-unshare", 0);
        let target_files: Vec<_> = fs::read_dir(&target_proj)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(target_files.len(), 2);
    }

    #[test]
    fn invalid_project_ids_are_rejected_for_sharing() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let err = share_code_project_one_way(source.path(), target.path(), "..").unwrap_err();
        assert!(err.contains("Invalid project id"), "got: {err}");
        let err = share_code_project_one_way(source.path(), target.path(), "../bad").unwrap_err();
        assert!(err.contains("Invalid project id"), "got: {err}");
    }

    fn write_desktop_code_session(
        sessions_root: &Path,
        device_id: &str,
        workspace_id: &str,
        session_local_id: &str,
        cwd: &str,
        last_activity_ms: i64,
    ) -> PathBuf {
        let dir = sessions_root.join(device_id).join(workspace_id);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("local_{session_local_id}.json"));
        let json = serde_json::json!({
            "sessionId": format!("local_{session_local_id}"),
            "cwd": cwd,
            "originCwd": cwd,
            "createdAt": last_activity_ms - 1000,
            "lastActivityAt": last_activity_ms,
            "title": format!("Session in {cwd}"),
        });
        fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();
        path
    }

    /// Write the two plain-JSON files Claude Desktop produces on every
    /// launch. Used by tests to fake the "I'm logged in as <acct> in
    /// org <org>" state without spinning up Desktop.
    fn write_desktop_login_files(data_dir: &Path, account_id: &str, org_id: &str) {
        fs::write(
            data_dir.join(COWORK_OPS_FILE),
            serde_json::to_string(&serde_json::json!({
                "ownerAccountId": account_id,
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            data_dir.join(EXTENSIONS_BLOCKLIST_FILE),
            serde_json::to_string(&serde_json::json!([
                {
                    "entries": [],
                    "lastUpdated": "2026-01-01T00:00:00.000Z",
                    "url": format!("https://claude.ai/api/organizations/{org_id}/dxt/blocklist"),
                }
            ]))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn scan_desktop_code_history_collects_recent_cwds_and_totals() {
        let data = tempfile::tempdir().unwrap();
        let sessions = data.path().join(DESKTOP_CODE_SESSIONS_DIR);
        write_desktop_code_session(&sessions, "dev1", "ws1", "aaaa", "/Users/me/projA", 1_700_000_000_000);
        write_desktop_code_session(&sessions, "dev1", "ws1", "bbbb", "/Users/me/projB", 1_700_000_005_000);
        write_desktop_code_session(&sessions, "dev1", "ws1", "cccc", "/Users/me/projA", 1_700_000_010_000);

        let stat = scan_desktop_code_history(&sessions).unwrap();
        assert!(stat.present);
        assert_eq!(stat.session_count, 3);
        assert_eq!(stat.last_activity_ms, 1_700_000_010_000);
        assert!(stat.total_bytes > 0);
        // projA was active most recently => first.
        assert_eq!(stat.recent_cwds.first().map(String::as_str), Some("/Users/me/projA"));
        assert!(stat.recent_cwds.contains(&"/Users/me/projB".to_string()));

        let primary = stat.primary_workspace.expect("workspace recorded");
        assert_eq!(primary.device_id, "dev1");
        assert_eq!(primary.workspace_id, "ws1");
    }

    #[test]
    fn scan_missing_desktop_code_history_returns_absent() {
        let data = tempfile::tempdir().unwrap();
        let stat = scan_desktop_code_history(&data.path().join(DESKTOP_CODE_SESSIONS_DIR)).unwrap();
        assert!(!stat.present);
        assert_eq!(stat.session_count, 0);
        assert!(stat.recent_cwds.is_empty());
        assert!(stat.primary_workspace.is_none());
    }

    #[test]
    fn empty_workspace_dir_is_still_recognised_as_primary() {
        let data = tempfile::tempdir().unwrap();
        let sessions = data.path().join(DESKTOP_CODE_SESSIONS_DIR);
        // Workspace exists on disk but has no session JSONs yet — this is
        // exactly the "freshly initialised" state we need before sharing.
        fs::create_dir_all(sessions.join("dev0").join("ws0")).unwrap();
        let stat = scan_desktop_code_history(&sessions).unwrap();
        assert!(stat.present);
        assert_eq!(stat.session_count, 0);
        let primary = stat.primary_workspace.expect("primary workspace recorded");
        assert_eq!(primary.device_id, "dev0");
        assert_eq!(primary.workspace_id, "ws0");
    }

    #[test]
    fn primary_workspace_is_the_most_recent_one() {
        let data = tempfile::tempdir().unwrap();
        let sessions = data.path().join(DESKTOP_CODE_SESSIONS_DIR);
        write_desktop_code_session(&sessions, "devOld", "wsOld", "aaaa", "/x", 1_000);
        write_desktop_code_session(&sessions, "devNew", "wsNew", "bbbb", "/y", 9_000);

        let stat = scan_desktop_code_history(&sessions).unwrap();
        let primary = stat.primary_workspace.unwrap();
        assert_eq!(primary.device_id, "devNew");
        assert_eq!(primary.workspace_id, "wsNew");
    }

    const SRC_ACCT: &str = "11111111-1111-1111-1111-111111111111";
    const SRC_ORG: &str = "22222222-2222-2222-2222-222222222222";
    const TGT_ACCT: &str = "33333333-3333-3333-3333-333333333333";
    const TGT_ORG: &str = "44444444-4444-4444-4444-444444444444";

    #[test]
    fn read_workspace_identity_from_json_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_workspace_identity(dir.path()).unwrap().is_none());
        write_desktop_login_files(dir.path(), SRC_ACCT, SRC_ORG);
        let id = read_workspace_identity(dir.path()).unwrap().unwrap();
        assert_eq!(id.device_id, SRC_ACCT);
        assert_eq!(id.workspace_id, SRC_ORG);
    }

    #[test]
    fn share_works_when_target_has_no_workspace_dir_yet() {
        // This is the user's real-world scenario: they logged into JUDY's
        // Desktop (so the JSON identity files exist) but never used the
        // Code panel, so `claude-code-sessions/<acct>/<org>/` is missing.
        // Sharing must succeed anyway by pre-creating the symlink.
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        // Source has 2 real sessions and is logged in.
        let src_sessions = source.path().join(DESKTOP_CODE_SESSIONS_DIR);
        write_desktop_code_session(&src_sessions, SRC_ACCT, SRC_ORG, "1111", "/work", 1_700_000_000_000);
        write_desktop_code_session(&src_sessions, SRC_ACCT, SRC_ORG, "2222", "/work", 1_700_000_001_000);
        write_desktop_login_files(source.path(), SRC_ACCT, SRC_ORG);

        // Target is logged in but has NEVER opened the Code panel (no
        // claude-code-sessions/ directory at all).
        write_desktop_login_files(target.path(), TGT_ACCT, TGT_ORG);
        assert!(!target.path().join(DESKTOP_CODE_SESSIONS_DIR).exists());

        let pre = list_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
        )
        .unwrap();
        assert!(!pre.shared);
        assert!(!pre.target_needs_bootstrap);
        assert!(!pre.source_needs_bootstrap);
        assert_eq!(pre.source.primary_workspace.as_ref().unwrap().device_id, SRC_ACCT);
        assert_eq!(pre.target.primary_workspace.as_ref().unwrap().device_id, TGT_ACCT);

        // Share — must NOT error even though target has no on-disk workspace.
        let summary = apply_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
            PairDesktopCodeHistoryChange { shared: true },
        )
        .unwrap();
        assert_eq!(summary.copied, 1);

        // Target's workspace dir was created and is a symlink to source's.
        let target_ws_path = target
            .path()
            .join(DESKTOP_CODE_SESSIONS_DIR)
            .join(TGT_ACCT)
            .join(TGT_ORG);
        let meta = fs::symlink_metadata(&target_ws_path).unwrap();
        assert!(meta.file_type().is_symlink(), "target <acct>/<org> must be a symlink");
        let resolved = fs::canonicalize(&target_ws_path).unwrap();
        let expected = fs::canonicalize(
            source
                .path()
                .join(DESKTOP_CODE_SESSIONS_DIR)
                .join(SRC_ACCT)
                .join(SRC_ORG),
        )
        .unwrap();
        assert_eq!(resolved, expected);

        // The whole-dir is NOT linked.
        let target_sessions_path = target.path().join(DESKTOP_CODE_SESSIONS_DIR);
        assert!(
            !fs::symlink_metadata(&target_sessions_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        // Through-link reads see source's 2 sessions.
        let post = list_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
        )
        .unwrap();
        assert!(post.shared);
        assert_eq!(post.direction, "source-to-target");
        assert_eq!(post.source.session_count, 2);
        assert_eq!(post.target.session_count, 2);

        // Live: writing a new session under source surfaces in target.
        write_desktop_code_session(&src_sessions, SRC_ACCT, SRC_ORG, "3333", "/work", 1_700_000_005_000);
        let post = list_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
        )
        .unwrap();
        assert_eq!(post.target.session_count, 3);
    }

    #[test]
    fn share_when_target_already_has_existing_workspace_backs_it_up() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        let src_sessions = source.path().join(DESKTOP_CODE_SESSIONS_DIR);
        write_desktop_code_session(&src_sessions, SRC_ACCT, SRC_ORG, "1111", "/work", 1_700_000_000_000);
        write_desktop_login_files(source.path(), SRC_ACCT, SRC_ORG);

        let tgt_sessions = target.path().join(DESKTOP_CODE_SESSIONS_DIR);
        write_desktop_code_session(&tgt_sessions, TGT_ACCT, TGT_ORG, "9999", "/lonely", 1_700_000_002_000);
        write_desktop_login_files(target.path(), TGT_ACCT, TGT_ORG);

        let summary = apply_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
            PairDesktopCodeHistoryChange { shared: true },
        )
        .unwrap();
        assert_eq!(summary.copied, 1);

        // Target's `<acct>/<org>` is now a symlink to source's; the original
        // is preserved under "Claude Multiprofile Backups".
        let target_ws_path = tgt_sessions.join(TGT_ACCT).join(TGT_ORG);
        assert!(fs::symlink_metadata(&target_ws_path).unwrap().file_type().is_symlink());
        let backups = target.path().join("Claude Multiprofile Backups");
        assert!(backups.exists());
    }

    #[test]
    fn share_errors_clearly_when_target_has_not_logged_in() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let src_sessions = source.path().join(DESKTOP_CODE_SESSIONS_DIR);
        write_desktop_code_session(&src_sessions, SRC_ACCT, SRC_ORG, "1111", "/work", 1_700_000_000_000);
        write_desktop_login_files(source.path(), SRC_ACCT, SRC_ORG);
        // Target has no JSON identity files at all (never launched).

        let pair = list_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
        )
        .unwrap();
        assert!(pair.target_needs_bootstrap);
        assert!(!pair.source_needs_bootstrap);

        let err = apply_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
            PairDesktopCodeHistoryChange { shared: true },
        )
        .unwrap_err();
        assert!(
            err.contains("Target profile hasn't completed Claude Desktop login"),
            "got: {err}",
        );
    }

    #[test]
    fn legacy_whole_dir_link_is_replaced_with_workspace_link() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        // Source has real sessions and is logged in.
        let src_sessions = source.path().join(DESKTOP_CODE_SESSIONS_DIR);
        write_desktop_code_session(&src_sessions, SRC_ACCT, SRC_ORG, "1111", "/work", 1_700_000_000_000);
        write_desktop_login_files(source.path(), SRC_ACCT, SRC_ORG);

        // Simulate the legacy whole-dir symlink that an earlier version of
        // this app would install.
        let tgt_sessions = target.path().join(DESKTOP_CODE_SESSIONS_DIR);
        symlink_path(&src_sessions, &tgt_sessions).unwrap();
        // Target is logged in (different account) but has no on-disk workspace.
        write_desktop_login_files(target.path(), TGT_ACCT, TGT_ORG);

        let pre = list_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
        )
        .unwrap();
        assert!(pre.legacy_whole_dir_link);

        // Apply share: legacy link is cleaned up and replaced with a
        // workspace-level link, all in one shot.
        let summary = apply_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
            PairDesktopCodeHistoryChange { shared: true },
        )
        .unwrap();
        assert_eq!(summary.copied, 1);

        let target_ws_path = tgt_sessions.join(TGT_ACCT).join(TGT_ORG);
        assert!(fs::symlink_metadata(&target_ws_path).unwrap().file_type().is_symlink());

        // Idempotent: re-applying the same share is a skip.
        let summary = apply_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
            PairDesktopCodeHistoryChange { shared: true },
        )
        .unwrap();
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.copied, 0);

        // Unshare: target's <acct>/<org> becomes an independent copy.
        let summary = apply_pair_desktop_code_history(
            source.path().to_string_lossy().to_string(),
            target.path().to_string_lossy().to_string(),
            PairDesktopCodeHistoryChange { shared: false },
        )
        .unwrap();
        assert_eq!(summary.copied, 1);
        assert!(!fs::symlink_metadata(&target_ws_path).unwrap().file_type().is_symlink());
    }

    #[test]
    fn list_code_installs_includes_default_when_dotclaude_exists() {
        // We cannot freely override HOME without affecting other tests in
        // parallel, so we just assert the function is well-formed and returns
        // something runnable. The integration smoke (manual GUI run) covers
        // the real ~/.claude detection.
        let installs = list_code_installs().unwrap();
        if let Some(default) = installs.iter().find(|i| i.kind == "default") {
            assert_eq!(default.id, "default");
        }
    }

    // ----- MCP server sharing -----

    fn write_desktop_config(dir: &Path, value: serde_json::Value) {
        fs::write(
            dir.join(DESKTOP_CONFIG_FILE),
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn list_pair_mcp_servers_unions_keys_and_detects_equal_values() {
        let src = tempfile::tempdir().unwrap();
        let tgt = tempfile::tempdir().unwrap();
        write_desktop_config(
            src.path(),
            serde_json::json!({
                "mcpServers": {
                    "shared": { "command": "npx", "args": ["foo"] },
                    "only-src": { "command": "echo" }
                }
            }),
        );
        write_desktop_config(
            tgt.path(),
            serde_json::json!({
                "mcpServers": {
                    "shared": { "command": "npx", "args": ["foo"] },
                    "only-tgt": { "command": "cat" }
                }
            }),
        );

        let rows = list_pair_mcp_servers(src.path(), tgt.path()).unwrap();
        // Sorted alphabetically.
        assert_eq!(
            rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
            vec!["only-src", "only-tgt", "shared"],
        );
        let shared = rows.iter().find(|r| r.name == "shared").unwrap();
        assert!(shared.source_present && shared.target_present);
        assert!(shared.copied);
        let only_src = rows.iter().find(|r| r.name == "only-src").unwrap();
        assert!(only_src.source_present && !only_src.target_present);
        assert!(!only_src.copied);
    }

    #[test]
    fn apply_pair_mcp_sharing_copies_and_removes() {
        let src = tempfile::tempdir().unwrap();
        let tgt = tempfile::tempdir().unwrap();
        write_desktop_config(
            src.path(),
            serde_json::json!({
                "mcpServers": { "foo": { "command": "npx" } }
            }),
        );
        write_desktop_config(
            tgt.path(),
            serde_json::json!({
                "mcpServers": { "bar": { "command": "cat" } }
            }),
        );

        // Copy "foo" from src to tgt.
        let summary = apply_pair_mcp_sharing(
            src.path().to_string_lossy().into(),
            tgt.path().to_string_lossy().into(),
            vec![PairMcpServerChange {
                name: "foo".to_string(),
                copied: true,
            }],
        )
        .unwrap();
        assert_eq!(summary.copied, 1);

        let tgt_cfg: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(tgt.path().join(DESKTOP_CONFIG_FILE)).unwrap())
                .unwrap();
        assert_eq!(
            tgt_cfg["mcpServers"]["foo"]["command"].as_str().unwrap(),
            "npx"
        );
        // Existing key should stay untouched.
        assert_eq!(
            tgt_cfg["mcpServers"]["bar"]["command"].as_str().unwrap(),
            "cat"
        );

        // Now remove "foo" from tgt.
        let summary = apply_pair_mcp_sharing(
            src.path().to_string_lossy().into(),
            tgt.path().to_string_lossy().into(),
            vec![PairMcpServerChange {
                name: "foo".to_string(),
                copied: false,
            }],
        )
        .unwrap();
        assert_eq!(summary.copied, 1);

        let tgt_cfg: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(tgt.path().join(DESKTOP_CONFIG_FILE)).unwrap())
                .unwrap();
        assert!(tgt_cfg["mcpServers"].get("foo").is_none());
        assert!(tgt_cfg["mcpServers"].get("bar").is_some());
    }

    #[test]
    fn apply_pair_mcp_sharing_no_op_when_already_equal() {
        let src = tempfile::tempdir().unwrap();
        let tgt = tempfile::tempdir().unwrap();
        let val = serde_json::json!({ "mcpServers": { "x": { "command": "y" } } });
        write_desktop_config(src.path(), val.clone());
        write_desktop_config(tgt.path(), val);

        let summary = apply_pair_mcp_sharing(
            src.path().to_string_lossy().into(),
            tgt.path().to_string_lossy().into(),
            vec![PairMcpServerChange {
                name: "x".to_string(),
                copied: true,
            }],
        )
        .unwrap();
        assert_eq!(summary.copied, 0);
        assert_eq!(summary.skipped, 1);
    }

    // ----- Preferences sharing -----

    #[test]
    fn list_pair_preferences_returns_allowlist_keys_in_both_scopes() {
        let src = tempfile::tempdir().unwrap();
        let tgt = tempfile::tempdir().unwrap();
        // UI scope
        fs::write(
            src.path().join(UI_CONFIG_FILE),
            r#"{"darkMode":"dark","scale":1}"#,
        )
        .unwrap();
        // Desktop pref scope
        write_desktop_config(
            src.path(),
            serde_json::json!({
                "preferences": { "menuBarEnabled": true, "chicagoEnabled": false }
            }),
        );

        let rows = list_pair_preferences(src.path(), tgt.path()).unwrap();
        // All allowlisted keys appear, even when target is empty.
        let ui_keys: Vec<_> = rows.iter().filter(|r| r.scope == "ui").map(|r| r.key.clone()).collect();
        assert!(ui_keys.contains(&"darkMode".to_string()));
        assert!(ui_keys.contains(&"scale".to_string()));
        let darkmode = rows.iter().find(|r| r.key == "darkMode").unwrap();
        assert!(darkmode.source_present);
        assert!(!darkmode.target_present);
        assert!(!darkmode.copied);
    }

    #[test]
    fn apply_pair_preferences_rejects_keys_outside_allowlist() {
        let src = tempfile::tempdir().unwrap();
        let tgt = tempfile::tempdir().unwrap();
        let err = set_pair_preference_copied(
            src.path(),
            tgt.path(),
            "bypassPermissionsOptInByAccount",
            "desktop_pref",
            true,
        )
        .unwrap_err();
        assert!(err.contains("allowlist"));
    }

    #[test]
    fn apply_pair_preferences_writes_only_target_key() {
        let src = tempfile::tempdir().unwrap();
        let tgt = tempfile::tempdir().unwrap();
        write_desktop_config(
            src.path(),
            serde_json::json!({
                "preferences": {
                    "menuBarEnabled": true,
                    "remoteToolsDeviceName": "mac-src"
                }
            }),
        );
        write_desktop_config(
            tgt.path(),
            serde_json::json!({
                "preferences": {
                    "remoteToolsDeviceName": "mac-tgt"
                }
            }),
        );

        let summary = apply_pair_preference_sharing(
            src.path().to_string_lossy().into(),
            tgt.path().to_string_lossy().into(),
            vec![PairPreferenceChange {
                key: "menuBarEnabled".to_string(),
                scope: "desktop_pref".to_string(),
                copied: true,
            }],
        )
        .unwrap();
        assert_eq!(summary.copied, 1);

        let tgt_cfg: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(tgt.path().join(DESKTOP_CONFIG_FILE)).unwrap())
                .unwrap();
        // Copied key landed.
        assert_eq!(tgt_cfg["preferences"]["menuBarEnabled"], serde_json::json!(true));
        // Untouched key kept its target-side value.
        assert_eq!(
            tgt_cfg["preferences"]["remoteToolsDeviceName"]
                .as_str()
                .unwrap(),
            "mac-tgt"
        );
    }

    // ----- Cowork Skills sharing -----

    fn write_skills_manifest(combo: &Path, value: serde_json::Value) {
        fs::create_dir_all(combo).unwrap();
        fs::write(
            combo.join(SKILLS_MANIFEST_FILE),
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn list_pair_cowork_skills_reports_bootstrap_when_no_combo() {
        let src = tempfile::tempdir().unwrap();
        let tgt = tempfile::tempdir().unwrap();
        let result = list_pair_cowork_skills(src.path(), tgt.path()).unwrap();
        assert!(result.source_needs_bootstrap);
        assert!(result.target_needs_bootstrap);
        assert!(result.rows.is_empty());
    }

    #[test]
    fn cowork_skill_share_symlinks_and_patches_manifest() {
        let src = tempfile::tempdir().unwrap();
        let tgt = tempfile::tempdir().unwrap();

        let src_combo = src.path().join(SKILLS_PLUGIN_REL).join("dev-a").join("acct-a");
        let tgt_combo = tgt.path().join(SKILLS_PLUGIN_REL).join("dev-b").join("acct-b");

        let entry = serde_json::json!({
            "skillId": "xlsx",
            "name": "xlsx",
            "description": "Excel handler",
            "creatorType": "anthropic",
            "enabled": true
        });
        write_skills_manifest(&src_combo, serde_json::json!({ "skills": [entry] }));
        write_skills_manifest(&tgt_combo, serde_json::json!({ "skills": [] }));

        // Create source skill folder content.
        let src_skill_dir = src_combo.join(SKILLS_SUBDIR).join("xlsx");
        fs::create_dir_all(&src_skill_dir).unwrap();
        fs::write(src_skill_dir.join("SKILL.md"), "hello").unwrap();

        // Share it.
        let summary = apply_pair_cowork_skills_sharing(
            src.path().to_string_lossy().into(),
            tgt.path().to_string_lossy().into(),
            vec![PairCoworkSkillChange {
                skill_id: "xlsx".to_string(),
                shared: true,
            }],
        )
        .unwrap();
        assert_eq!(summary.copied, 1);

        // Target's skills/xlsx should now be a symlink at source's.
        let tgt_skill_dir = tgt_combo.join(SKILLS_SUBDIR).join("xlsx");
        let link_meta = fs::symlink_metadata(&tgt_skill_dir).unwrap();
        assert!(link_meta.file_type().is_symlink());

        // Target manifest should contain the source entry.
        let tgt_manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(tgt_combo.join(SKILLS_MANIFEST_FILE)).unwrap(),
        )
        .unwrap();
        let skills = tgt_manifest["skills"].as_array().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0]["skillId"].as_str().unwrap(), "xlsx");
        assert!(tgt_manifest.get("lastUpdated").is_some());

        // listPairCoworkSkills should now report shared: true.
        let result = list_pair_cowork_skills(src.path(), tgt.path()).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert!(result.rows[0].shared);

        // Unshare and confirm cleanup.
        let summary = apply_pair_cowork_skills_sharing(
            src.path().to_string_lossy().into(),
            tgt.path().to_string_lossy().into(),
            vec![PairCoworkSkillChange {
                skill_id: "xlsx".to_string(),
                shared: false,
            }],
        )
        .unwrap();
        assert_eq!(summary.copied, 1);
        assert!(fs::symlink_metadata(&tgt_skill_dir).is_err());
        let tgt_manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(tgt_combo.join(SKILLS_MANIFEST_FILE)).unwrap(),
        )
        .unwrap();
        assert!(tgt_manifest["skills"].as_array().unwrap().is_empty());
    }

    fn make_cell(install_id: &str, present: bool, digest: Option<&str>, link: Option<&str>) -> LibraryCell {
        LibraryCell {
            install_id: install_id.into(),
            install_name: install_id.into(),
            data_dir: "/tmp".into(),
            kind: "profile".into(),
            state: String::new(),
            present,
            detail: None,
            digest: digest.map(String::from),
            link_target_digest: link.map(String::from),
        }
    }

    #[test]
    fn symlink_share_states_partial_group_and_transitive() {
        // A real source dir + two symlinks into it (B,C), plus an independent D
        // and an absent E. All of A/B/C must read "shared".
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let c = tempfile::tempdir().unwrap();
        let d = tempfile::tempdir().unwrap();
        let pa = a.path().join("x");
        fs::create_dir_all(&pa).unwrap();
        let pb = b.path().join("x");
        std::os::unix::fs::symlink(&pa, &pb).unwrap();
        let pc = c.path().join("x");
        std::os::unix::fs::symlink(&pa, &pc).unwrap();
        let pd = d.path().join("x");
        fs::create_dir_all(&pd).unwrap(); // real, no link relationship
        let pe = a.path().join("absent");

        let paths = vec![pa, pb, pc, pd, pe];
        let present = vec![true, true, true, true, false];
        let s = symlink_share_states(&paths, &present);
        assert_eq!(s[0], "shared", "real source");
        assert_eq!(s[1], "shared", "symlink B");
        assert_eq!(s[2], "shared", "symlink C (transitive group)");
        assert_eq!(s[3], "independent", "unrelated real dir");
        assert_eq!(s[4], "absent");
    }

    #[test]
    fn share_claude_sessions_creates_absent_target_then_reads_shared() {
        // Regression: sharing the whole sessions dir into a derived-but-absent
        // account (the JUDY case, ~/.claude-judy) must create the parent dir,
        // create the symlink, and afterward read "shared" on both sides — not
        // silently fail with ENOENT and leave the cell "independent".
        let src = tempfile::tempdir().unwrap();
        let src_projects = src.path().join("projects");
        fs::create_dir_all(&src_projects).unwrap();
        fs::write(src_projects.join("a.jsonl"), "{}\n").unwrap();

        let base = tempfile::tempdir().unwrap();
        let target_config = base.path().join(".claude-judy"); // does NOT exist yet
        assert!(!target_config.exists());

        let created = share_claude_sessions(src.path(), &target_config).unwrap();
        assert!(created, "share must report it created the link");

        let target_projects = target_config.join("projects");
        let md = fs::symlink_metadata(&target_projects).unwrap();
        assert!(md.file_type().is_symlink(), "target projects/ must be a symlink");
        assert!(path_points_to(&target_projects, &src_projects));

        let states = symlink_share_states(
            &[src_projects.clone(), target_projects.clone()],
            &[true, true],
        );
        assert_eq!(states, vec!["shared", "shared"], "both sides read shared");

        // Idempotent re-share is a no-op (already linked).
        assert!(!share_claude_sessions(src.path(), &target_config).unwrap());
    }

    /// END-TO-END: the EXACT path the GUI runs — toggle whole-sessions share onto
    /// a derived-but-absent account (JUDY), through the real apply command, then
    /// re-read the library and assert BOTH cells report "shared". Proves the
    /// control + detection chain works on the JUDY scenario, not just the helper.
    #[test]
    fn e2e_share_whole_sessions_onto_judy_reads_shared() {
        let _env = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));

        // ~/.claude/projects/<proj>/<session>.jsonl — Default has real sessions.
        let proj = home.path().join(".claude").join("projects").join("-Users-x-demo");
        fs::create_dir_all(&proj).unwrap();
        fs::write(
            proj.join("s1.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/Users/x/demo\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
        )
        .unwrap();
        // Registry with a JUDY profile that has NO code dir (code:null) — the
        // exact shape of the user's profiles.json.
        let reg = home.path().join(".config").join("claude-multiprofile");
        fs::create_dir_all(&reg).unwrap();
        fs::write(
            reg.join("profiles.json"),
            r#"{"version":1,"profiles":[{"name":"judy","type":"both","desktop":null,"code":null,"codex":null,"createdAt":"2026-01-01T00:00:00Z"}]}"#,
        )
        .unwrap();

        // BEFORE: JUDY's whole-sessions cell is absent (nothing shared).
        let before = list_claude_sessions_library().unwrap();
        let all_before = before.iter().find(|r| r.id == CODEX_ALL_SESSIONS_ID).unwrap();
        let judy_before = all_before.cells.iter().find(|c| c.install_id == "profile:judy").unwrap();
        assert_eq!(judy_before.state, "absent", "JUDY starts unshared");

        // ACT: the real GUI apply — toggle "All Claude sessions" share onto JUDY.
        apply_library_changes(
            "claude_sessions".to_string(),
            vec![LibraryCellChange {
                row_id: CODEX_ALL_SESSIONS_ID.to_string(),
                target_install_id: "profile:judy".to_string(),
                wants: true,
                source_install_id: None,
            }],
        )
        .unwrap();

        // AFTER: ~/.claude-judy/projects is a live symlink AND both cells read shared.
        let judy_link = home.path().join(".claude-judy").join("projects");
        assert!(
            fs::symlink_metadata(&judy_link).unwrap().file_type().is_symlink(),
            "apply must have created the projects symlink"
        );
        let after = list_claude_sessions_library().unwrap();
        let all_after = after.iter().find(|r| r.id == CODEX_ALL_SESSIONS_ID).unwrap();
        let judy_after = all_after.cells.iter().find(|c| c.install_id == "profile:judy").unwrap();
        let def_after = all_after.cells.iter().find(|c| c.install_id == "default").unwrap();
        assert_eq!(judy_after.state, "shared", "JUDY reads shared after the toggle");
        assert_eq!(def_after.state, "shared", "Default reads shared after the toggle");

        // restore env for any sibling tests
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    /// END-TO-END: un-sharing whole sessions by clicking the SOURCE cell (the real
    /// dir others link into) must actually detach the linkers — previously a
    /// silent no-op that left both cells stuck on "shared".
    #[test]
    fn e2e_unshare_sessions_from_source_cell_detaches_linkers() {
        let _env = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));

        let proj = home.path().join(".claude").join("projects").join("-Users-x-demo");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("s1.jsonl"), "{\"cwd\":\"/Users/x/demo\"}\n").unwrap();
        let reg = home.path().join(".config").join("claude-multiprofile");
        fs::create_dir_all(&reg).unwrap();
        fs::write(
            reg.join("profiles.json"),
            r#"{"version":1,"profiles":[{"name":"judy","type":"both","desktop":null,"code":null,"codex":null,"createdAt":"2026-01-01T00:00:00Z"}]}"#,
        )
        .unwrap();

        let toggle = |target: &str, wants: bool| {
            apply_library_changes(
                "claude_sessions".to_string(),
                vec![LibraryCellChange {
                    row_id: CODEX_ALL_SESSIONS_ID.to_string(),
                    target_install_id: target.to_string(),
                    wants,
                    source_install_id: None,
                }],
            )
            .unwrap();
        };

        toggle("profile:judy", true); // share default -> judy
        let judy_link = home.path().join(".claude-judy").join("projects");
        assert!(fs::symlink_metadata(&judy_link).unwrap().file_type().is_symlink());

        toggle("default", false); // un-share from the SOURCE cell

        let md = fs::symlink_metadata(&judy_link).unwrap();
        assert!(!md.file_type().is_symlink(), "un-share from source must detach the linker");
        assert!(
            judy_link.join("-Users-x-demo").join("s1.jsonl").exists(),
            "detached dir keeps the sessions content"
        );
        let after = list_claude_sessions_library().unwrap();
        let all = after.iter().find(|r| r.id == CODEX_ALL_SESSIONS_ID).unwrap();
        assert_eq!(all.cells.iter().find(|c| c.install_id == "profile:judy").unwrap().state, "independent");
        assert_eq!(all.cells.iter().find(|c| c.install_id == "default").unwrap().state, "independent");

        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn compute_row_states_copy_detects_diverged_when_digests_disagree() {
        let mut row = LibraryRow {
            id: "mcp-foo".into(),
            label: "mcp-foo".into(),
            description: None,
            cells: vec![
                make_cell("a", true, Some("hashA"), None),
                make_cell("b", true, Some("hashA"), None),
                make_cell("c", true, Some("hashB"), None),
                make_cell("d", false, None, None),
            ],
            interactive: true,
            group: None,
        };
        compute_row_states(&mut row);
        // a and b have the same digest, but c diverges — everybody present
        // that has a sibling is "diverged" because at least one other present
        // cell has a different digest.
        assert_eq!(row.cells[0].state, "diverged");
        assert_eq!(row.cells[1].state, "diverged");
        assert_eq!(row.cells[2].state, "diverged");
        assert_eq!(row.cells[3].state, "absent");
    }

    #[test]
    fn compute_row_states_copy_marks_unique_present_as_independent() {
        let mut row = LibraryRow {
            id: "pref-x".into(),
            label: "x".into(),
            description: None,
            cells: vec![
                make_cell("a", true, Some("hashA"), None),
                make_cell("b", false, None, None),
            ],
            interactive: true,
            group: None,
        };
        compute_row_states(&mut row);
        assert_eq!(row.cells[0].state, "independent");
        assert_eq!(row.cells[1].state, "absent");
    }

    #[test]
    fn compute_row_states_copy_marks_two_matching_as_copied() {
        let mut row = LibraryRow {
            id: "mcp-bar".into(),
            label: "mcp-bar".into(),
            description: None,
            cells: vec![
                make_cell("a", true, Some("h"), None),
                make_cell("b", true, Some("h"), None),
                make_cell("c", false, None, None),
            ],
            interactive: true,
            group: None,
        };
        compute_row_states(&mut row);
        assert_eq!(row.cells[0].state, "copied");
        assert_eq!(row.cells[1].state, "copied");
        assert_eq!(row.cells[2].state, "absent");
    }

    #[test]
    fn codex_data_dir_and_launcher_paths_mirror_claude_naming() {
        // Codex profiles use the same name-casing as Claude Desktop, just with
        // the Codex- / "Codex " prefixes (e.g. "work" -> "WORK").
        let cased = title_case("work");
        let data = codex_data_dir_for("work").unwrap();
        assert!(data
            .to_string_lossy()
            .contains(&format!("Application Support/Codex-{cased}")));
        let launcher = codex_launcher_path_for("work").unwrap();
        assert!(launcher
            .to_string_lossy()
            .ends_with(&format!("Applications/Codex {cased}.app")));
        // CODEX_HOME mirrors Claude Code's ~/.claude-<name> dotdir convention.
        let home = codex_home_dir_for("WORK").unwrap();
        assert!(home.to_string_lossy().ends_with("/.codex-work"));
    }

    #[test]
    fn ensure_codex_file_auth_backend_sets_file_and_preserves_other_keys() {
        let home = tempfile::tempdir().unwrap();
        // Pre-existing config with an unrelated key and the wrong store mode.
        fs::write(
            home.path().join("config.toml"),
            "model = \"gpt-5\"\ncli_auth_credentials_store = \"auto\"\n",
        )
        .unwrap();
        ensure_codex_file_auth_backend(home.path()).unwrap();
        let out = fs::read_to_string(home.path().join("config.toml")).unwrap();
        let doc: toml_edit::ImDocument<String> = out.parse().unwrap();
        assert_eq!(
            doc.get("cli_auth_credentials_store").and_then(|v| v.as_str()),
            Some("file"),
            "store mode must be pinned to file (keychain backend is global)"
        );
        assert_eq!(
            doc.get("model").and_then(|v| v.as_str()),
            Some("gpt-5"),
            "unrelated keys must be preserved"
        );
        // Idempotent: a second call is a no-op and still valid.
        ensure_codex_file_auth_backend(home.path()).unwrap();
        let out2 = fs::read_to_string(home.path().join("config.toml")).unwrap();
        assert!(out2.contains("cli_auth_credentials_store = \"file\""));
    }

    #[test]
    fn ensure_codex_file_auth_backend_creates_config_when_absent() {
        let home = tempfile::tempdir().unwrap();
        // No config.toml yet — the helper should create one.
        ensure_codex_file_auth_backend(home.path()).unwrap();
        let out = fs::read_to_string(home.path().join("config.toml")).unwrap();
        assert!(out.contains("cli_auth_credentials_store = \"file\""));
    }

    #[test]
    fn codex_launcher_script_pins_codex_home_but_claude_does_not() {
        let data = Path::new("/tmp/Codex-WORK");
        let app = Path::new("/Applications/Codex.app");
        let home = Path::new("/Users/me/.codex-work");
        // Codex: must carry --env CODEX_HOME, placed BEFORE --args.
        let codex = build_launch_applescript(data, app, Some(home));
        assert!(codex.contains("--env CODEX_HOME='/Users/me/.codex-work'"));
        let env_at = codex.find("--env").unwrap();
        let args_at = codex.find("--args").unwrap();
        assert!(env_at < args_at, "--env must precede --args");
        // Claude: no CODEX_HOME — user-data-dir alone isolates Electron.
        let claude = build_launch_applescript(data, app, None);
        assert!(!claude.contains("CODEX_HOME"));
        assert!(claude.contains("--user-data-dir="));
    }

    #[test]
    fn write_json_atomically_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"old":true}"#).unwrap();
        write_json_atomically(&path, &serde_json::json!({ "new": 42 })).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["new"], serde_json::json!(42));
        assert!(v.get("old").is_none());
    }

    #[test]
    fn mcp_json_to_toml_roundtrip_preserves_command_args_and_env() {
        // A typical Claude Code stdio MCP server, including a nested env object.
        let server = serde_json::json!({
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            "env": { "API_KEY": "abc123", "DEBUG": "1" }
        });
        let obj = server.as_object().unwrap();

        // JSON object -> real TOML table (env should become a sub-table).
        let table = json_object_to_toml_table(obj);
        let mut doc = toml_edit::DocumentMut::new();
        let mut servers = toml_edit::Table::new();
        servers.set_implicit(true);
        servers.insert("fs", toml_edit::Item::Table(table));
        doc["mcp_servers"] = toml_edit::Item::Table(servers);
        let rendered = doc.to_string();
        assert!(rendered.contains("[mcp_servers.fs]"));
        assert!(rendered.contains("[mcp_servers.fs.env]"));

        // TOML -> JSON should reproduce the original config exactly.
        let parsed: toml_edit::ImDocument<String> = rendered.parse().unwrap();
        let back = toml_item_to_json(parsed.get("mcp_servers").unwrap().get("fs").unwrap());
        assert_eq!(
            mcp_value_digest(&server),
            mcp_value_digest(&back),
            "round-tripped config must be digest-stable so the cell reads as `copied`"
        );
        assert_eq!(back["command"], serde_json::json!("npx"));
        assert_eq!(back["env"]["API_KEY"], serde_json::json!("abc123"));
    }

    #[test]
    fn symlink_share_states_real_source_plus_symlink_reads_shared_on_both() {
        // Topology that apply_codex_skill_share creates: profile A holds a real
        // skill dir, profile B symlinks into it. Both must read "shared".
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let c = tempfile::tempdir().unwrap();
        let a_skill = a.path().join("x");
        fs::create_dir_all(&a_skill).unwrap();
        let b_skill = b.path().join("x");
        std::os::unix::fs::symlink(&a_skill, &b_skill).unwrap();
        let c_skill = c.path().join("x"); // absent in C

        let paths = vec![a_skill.clone(), b_skill.clone(), c_skill.clone()];
        let present = vec![true, true, false];
        let states = symlink_share_states(&paths, &present);
        assert_eq!(states[0], "shared", "real source must read shared");
        assert_eq!(states[1], "shared", "symlink target must read shared");
        assert_eq!(states[2], "absent", "missing column is absent");

        // A lone real dir with no link partner is independent, not shared.
        let lone = vec![a_skill, c.path().join("y")];
        let states2 = symlink_share_states(&lone, &[true, false]);
        assert_eq!(states2[0], "independent");
    }

    #[test]
    fn codex_sessions_share_symlinks_then_make_independent_copies_back() {
        let src_home = tempfile::tempdir().unwrap();
        let tgt_home = tempfile::tempdir().unwrap();
        // Source has a real sessions/ dir with a rollout; target has its own.
        let src_sessions = src_home.path().join("sessions");
        fs::create_dir_all(&src_sessions).unwrap();
        fs::write(src_sessions.join("rollout-a.jsonl"), "{}").unwrap();
        let tgt_sessions = tgt_home.path().join("sessions");
        fs::create_dir_all(&tgt_sessions).unwrap();
        fs::write(tgt_sessions.join("rollout-old.jsonl"), "{}").unwrap();

        // Share: target/sessions becomes a symlink to source/sessions.
        assert!(share_codex_sessions(src_home.path(), tgt_home.path()).unwrap());
        assert!(tgt_sessions
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(tgt_sessions.join("rollout-a.jsonl").exists(), "sees source content");
        // Idempotent: second share is a no-op.
        assert!(!share_codex_sessions(src_home.path(), tgt_home.path()).unwrap());

        // Make independent resolves the symlink target itself (no source arg),
        // so it copies back the EXACT content the target was showing.
        assert!(make_codex_sessions_independent(tgt_home.path()).unwrap());
        assert!(!tgt_sessions
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(tgt_sessions.is_dir());
        assert!(tgt_sessions.join("rollout-a.jsonl").exists());
    }

    #[test]
    fn make_codex_sessions_independent_copies_the_actual_link_target_not_a_guess() {
        // 3 accounts: target links to C (not the "first other" B). The undo must
        // copy back C's content, proving it reads the real symlink target.
        let b = tempfile::tempdir().unwrap();
        let c = tempfile::tempdir().unwrap();
        let tgt = tempfile::tempdir().unwrap();
        let c_sessions = c.path().join("sessions");
        fs::create_dir_all(&c_sessions).unwrap();
        fs::write(c_sessions.join("from-c.jsonl"), "{}").unwrap();
        let b_sessions = b.path().join("sessions");
        fs::create_dir_all(&b_sessions).unwrap();
        fs::write(b_sessions.join("from-b.jsonl"), "{}").unwrap();
        // target/sessions -> C/sessions
        std::os::unix::fs::symlink(&c_sessions, tgt.path().join("sessions")).unwrap();

        assert!(make_codex_sessions_independent(tgt.path()).unwrap());
        let tgt_sessions = tgt.path().join("sessions");
        assert!(tgt_sessions.join("from-c.jsonl").exists(), "copied C's content");
        assert!(!tgt_sessions.join("from-b.jsonl").exists(), "did NOT copy B's content");
    }

    /// DEFENSE IN DEPTH: share_codex_sessions must REFUSE a link that would form a
    /// cycle (target already resolves to the source), and must do so BEFORE any
    /// backup — so a refused op never displaces real data.
    #[test]
    fn share_codex_sessions_refuses_cycle_before_backup() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        // A has the real sessions; B/sessions is a symlink -> A/sessions.
        let a_sessions = a.path().join("sessions");
        fs::create_dir_all(&a_sessions).unwrap();
        fs::write(a_sessions.join("keep.jsonl"), "{}").unwrap();
        std::os::unix::fs::symlink(&a_sessions, b.path().join("sessions")).unwrap();

        // Ask A to link to B — but B already resolves to A => would cycle.
        let res = share_codex_sessions(b.path(), a.path());
        assert!(res.is_err(), "must refuse the cycle");
        assert!(res.unwrap_err().contains("cycle or self-link"));
        // No backup created, A's real content untouched (refused before backup).
        assert!(!a.path().join("Claude Multiprofile Backups").exists());
        assert!(a_sessions.is_dir() && a_sessions.join("keep.jsonl").exists());

        // Self-link (A onto itself) is likewise refused, A intact.
        let res2 = share_codex_sessions(a.path(), a.path());
        assert!(res2.is_err(), "self-link refused");
        assert!(a_sessions.join("keep.jsonl").exists());
    }

    #[test]
    fn parse_user_data_dir_handles_spaces_and_flags() {
        assert_eq!(
            parse_user_data_dir("/x/Codex --user-data-dir=/Users/me/Library/Application Support/Codex-JUDY --enable-foo"),
            Some(PathBuf::from("/Users/me/Library/Application Support/Codex-JUDY")),
        );
        // Last token (no trailing flag).
        assert_eq!(
            parse_user_data_dir("/x/Codex --user-data-dir=/Users/me/Library/Application Support/Codex"),
            Some(PathBuf::from("/Users/me/Library/Application Support/Codex")),
        );
        assert_eq!(parse_user_data_dir("/x/Codex --no-flag"), None);
    }

    #[test]
    fn codex_data_dir_from_lsof_resolves_profile_and_excludes_codexbar() {
        let base = Path::new("/Users/me/Library/Application Support");
        // A profile process: its open Codex-JUDY data dir wins.
        let raw = "p123\nn/Users/me/Library/Application Support/Codex-JUDY/Local State\nn/etc/hosts\n";
        assert_eq!(codex_data_dir_from_lsof(raw, base), Some(base.join("Codex-JUDY")));
        // Default process.
        let raw_def = "n/Users/me/Library/Application Support/Codex/Cookies\n";
        assert_eq!(codex_data_dir_from_lsof(raw_def, base), Some(base.join("Codex")));
        // The unrelated CodexBar app must NOT be attributed to Codex/profiles.
        let raw_bar = "n/Users/me/Library/Application Support/CodexBar/state.json\n";
        assert_eq!(codex_data_dir_from_lsof(raw_bar, base), None);
    }

    /// END-TO-END regression for the data-loss incident: toggling SHARE on BOTH
    /// Codex accounts in one Apply must NOT form a circular symlink. The batch
    /// collapses to a single canonical hub = the RICHEST account; the other links
    /// to it; no rollout is lost.
    #[test]
    fn e2e_codex_share_both_cells_no_cycle_richest_wins() {
        let _env = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        let prev_codex = std::env::var_os("CODEX_HOME");
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
        std::env::remove_var("CODEX_HOME"); // default => ~/.codex

        // Default (~/.codex) is the RICHER account: 5 rollouts. JUDY: 1.
        let def_sessions = home.path().join(".codex").join("sessions").join("2026").join("06");
        fs::create_dir_all(&def_sessions).unwrap();
        for i in 0..5 {
            fs::write(def_sessions.join(format!("rollout-{i}.jsonl")), "{}").unwrap();
        }
        let judy_sessions = home.path().join(".codex-judy").join("sessions").join("2026");
        fs::create_dir_all(&judy_sessions).unwrap();
        fs::write(judy_sessions.join("rollout-j.jsonl"), "{}").unwrap();

        // Registry: a JUDY codex profile (non-null codex) so it's a column.
        let reg = home.path().join(".config").join("claude-multiprofile");
        fs::create_dir_all(&reg).unwrap();
        fs::write(
            reg.join("profiles.json"),
            r#"{"version":1,"profiles":[{"name":"judy","type":"both","desktop":null,"code":null,"codex":{"dataDir":"x","codexHome":"x","launcherPath":"x","codexAppPath":"x"},"createdAt":"2026-01-01T00:00:00Z"}]}"#,
        )
        .unwrap();

        // ACT: the exact incident batch — share BOTH cells at once.
        apply_library_changes(
            "codex_sessions".to_string(),
            vec![
                LibraryCellChange {
                    row_id: CODEX_ALL_SESSIONS_ID.to_string(),
                    target_install_id: "default".to_string(),
                    wants: true,
                    source_install_id: None,
                },
                LibraryCellChange {
                    row_id: CODEX_ALL_SESSIONS_ID.to_string(),
                    target_install_id: "profile:judy".to_string(),
                    wants: true,
                    source_install_id: None,
                },
            ],
        )
        .unwrap();

        let def_dir = home.path().join(".codex").join("sessions");
        let judy_dir = home.path().join(".codex-judy").join("sessions");

        // (1) The richest (default, 5) is the single REAL hub; JUDY is a symlink.
        assert!(
            !fs::symlink_metadata(&def_dir).unwrap().file_type().is_symlink(),
            "default sessions stays the real hub"
        );
        assert_eq!(count_jsonl_under(&def_dir), 5, "hub keeps all 5 rollouts");
        assert!(
            fs::symlink_metadata(&judy_dir).unwrap().file_type().is_symlink(),
            "judy sessions became a symlink to the hub"
        );

        // (2) No cycle: both canonicalize and resolve to the SAME real dir.
        let cdef = fs::canonicalize(&def_dir).unwrap();
        let cjudy = fs::canonicalize(&judy_dir).unwrap();
        assert_eq!(cdef, cjudy, "both point at one shared real pool, no loop");

        // (3) Zero rollouts lost: hub (5) + JUDY's displaced backup (1) == 6.
        let judy_backup = home.path().join(".codex-judy").join("Claude Multiprofile Backups");
        assert_eq!(count_jsonl_under(&judy_backup), 1, "judy's 1 rollout preserved in backup");
        assert!(
            !home.path().join(".codex").join("Claude Multiprofile Backups").exists(),
            "the hub was never displaced"
        );

        // (4) Both cells read shared.
        let rows = list_codex_sessions_library().unwrap();
        let all = rows.iter().find(|r| r.id == CODEX_ALL_SESSIONS_ID).unwrap();
        for id in ["default", "profile:judy"] {
            let c = all.cells.iter().find(|c| c.install_id == id).unwrap();
            assert_eq!(c.state, "shared", "{id} reads shared after the safe share");
        }

        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        if let Some(v) = prev_codex {
            std::env::set_var("CODEX_HOME", v);
        }
    }

    #[test]
    fn remove_data_dir_refuses_dangerous_paths() {
        // Refuses home, root, and anything not safely under home — without
        // deleting anything (these all error before any removal).
        let _env = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = home_dir().unwrap();
        assert!(remove_data_dir(&home).is_err(), "home refused");
        assert!(remove_data_dir(Path::new("/")).is_err(), "root refused");
        // A real dir outside home (tempdir lives under /var or /tmp) → refused.
        let outside = tempfile::tempdir().unwrap();
        assert!(
            remove_data_dir(outside.path()).is_err(),
            "outside-home refused"
        );
        // A non-existent path is a no-op (idempotent), not an error.
        assert!(remove_data_dir(&home.join(".claudex-does-not-exist-xyz")).is_ok());

        // A REAL 1-component dir under HOME that is NOT one of our managed agent
        // homes must be refused (guards ~/Documents, ~/.ssh, …). Create+refuse+
        // verify-still-there, then clean up.
        let shallow = home.join(".claudex-test-shallow-refuse-me");
        fs::create_dir_all(&shallow).unwrap();
        let res = remove_data_dir(&shallow);
        let still = shallow.exists();
        let _ = fs::remove_dir_all(&shallow); // cleanup regardless
        assert!(res.is_err(), "1-deep non-managed dir must be refused");
        assert!(still, "refused dir must NOT have been deleted");

        // But a managed ~/.codex-<name> (1-deep, allowlisted) IS removable.
        let managed = home.join(".codex-claudex-test-allow-me");
        fs::create_dir_all(&managed).unwrap();
        assert!(remove_data_dir(&managed).is_ok());
        assert!(!managed.exists(), "managed dotdir should be removed");
    }

    #[test]
    fn guarded_file_path_confines_to_claude_codex_roots() {
        let _env = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = home_dir().unwrap();
        // A real managed root must exist for canonicalize to succeed; create one.
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).ok();
        // Accept a file under ~/.claude.
        assert!(guarded_file_path(&claude.join("CLAUDE.md").to_string_lossy()).is_ok());
        // Refuse ~/.ssh and friends (parent under home but not a managed root).
        let ssh = home.join(".ssh");
        fs::create_dir_all(&ssh).ok();
        assert!(guarded_file_path(&ssh.join("id_rsa").to_string_lossy()).is_err());
        // Refuse a file directly in $HOME.
        assert!(guarded_file_path(&home.join("secret.txt").to_string_lossy()).is_err());

        // Project-level memory: a CLAUDE.md at a normal (non-hidden) project root
        // IS allowed (that's where most CLAUDE.md actually live).
        let proj = home.join("claudex_guard_test_proj");
        fs::create_dir_all(&proj).ok();
        assert!(guarded_file_path(&proj.join("CLAUDE.md").to_string_lossy()).is_ok());
        // …but the memory carve-out must NOT reach into hidden dot-dirs.
        assert!(guarded_file_path(&ssh.join("CLAUDE.md").to_string_lossy()).is_err());
        // …and a memory file that is itself a symlink is refused (no
        // CLAUDE.md → ~/.ssh/id_rsa exfiltration).
        let link = proj.join("AGENTS.md");
        std::os::unix::fs::symlink(ssh.join("id_rsa"), &link).ok();
        assert!(guarded_file_path(&link.to_string_lossy()).is_err());
        fs::remove_dir_all(&proj).ok();
    }

    #[test]
    fn char_truncate_is_utf8_safe() {
        // Multibyte CJK + emoji must not panic when truncated at a byte boundary
        // that splits a char.
        let s = "日本語テキストの長い内容😀😀😀".repeat(20);
        let t = char_truncate(&s, 5);
        assert!(t.chars().count() <= 6); // 5 + the ellipsis
        assert!(t.ends_with('…'));
        assert_eq!(char_truncate("short", 60), "short");
    }

    #[test]
    fn memory_cross_filename_symlink_reads_shared_on_both() {
        // Cross-platform memory: AGENTS.md (Codex) symlinked to CLAUDE.md (Claude).
        // The filename differs but the share-state detection must still see both
        // as shared (single-file analog of the skill symlink topology).
        let claude = tempfile::tempdir().unwrap();
        let codex = tempfile::tempdir().unwrap();
        let claude_md = claude.path().join("CLAUDE.md");
        fs::write(&claude_md, "# memory\nremember things\n").unwrap();
        let agents_md = codex.path().join("AGENTS.md");
        std::os::unix::fs::symlink(&claude_md, &agents_md).unwrap();

        let paths = vec![claude_md.clone(), agents_md.clone()];
        let present = vec![memory_present(&claude_md), memory_present(&agents_md)];
        assert_eq!(present, vec![true, true], "both memory files present");
        let states = symlink_share_states(&paths, &present);
        assert_eq!(states[0], "shared", "CLAUDE.md source reads shared");
        assert_eq!(states[1], "shared", "AGENTS.md symlink reads shared");
        // A lone CLAUDE.md with no partner is independent.
        assert_eq!(
            symlink_share_states(&[claude_md], &[true])[0],
            "independent"
        );
    }

    #[test]
    fn codex_mcp_write_read_remove_at_explicit_path_roundtrips() {
        // The core of between-Codex-profile MCP sharing: copy a server into a
        // target profile's config.toml, read it back digest-stable, remove it.
        let home = tempfile::tempdir().unwrap();
        let cfg = home.path().join("config.toml");
        // Pre-existing unrelated content must survive.
        fs::write(&cfg, "model = \"gpt-5\"\n").unwrap();
        let server = serde_json::json!({
            "command": "npx",
            "args": ["-y", "some-mcp"],
            "env": { "TOKEN": "x" }
        });

        write_codex_mcp_server_at(&cfg, "srv", &server).unwrap();
        let back = read_codex_mcp_at(&cfg);
        assert!(back.contains_key("srv"));
        assert_eq!(
            mcp_value_digest(&server),
            mcp_value_digest(&back["srv"]),
            "copied server must be digest-stable across the TOML round-trip"
        );
        let raw = fs::read_to_string(&cfg).unwrap();
        assert!(raw.contains("model = \"gpt-5\""), "unrelated keys preserved");
        assert!(raw.contains("[mcp_servers.srv.env]"));

        assert!(remove_codex_mcp_server_at(&cfg, "srv").unwrap());
        assert!(!read_codex_mcp_at(&cfg).contains_key("srv"));
        // Removing a missing server is a no-op (false), not an error.
        assert!(!remove_codex_mcp_server_at(&cfg, "srv").unwrap());
    }

    #[test]
    fn canonical_json_string_is_key_order_independent() {
        let a = serde_json::json!({ "command": "x", "args": ["1", "2"] });
        let b = serde_json::json!({ "args": ["1", "2"], "command": "x" });
        assert_eq!(mcp_value_digest(&a), mcp_value_digest(&b));
        // Array order still matters — these must differ.
        let c = serde_json::json!({ "command": "x", "args": ["2", "1"] });
        assert_ne!(mcp_value_digest(&a), mcp_value_digest(&c));
    }

}

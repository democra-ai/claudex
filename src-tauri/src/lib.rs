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

        if file_type.is_dir() {
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

fn build_launch_applescript(data_dir: &Path, claude_app_path: &Path) -> String {
    let safe_app = shell_quote_single(claude_app_path);
    let safe_dir = shell_quote_single(data_dir);
    format!(
        "do shell script \"open -n -a '{}' --args --user-data-dir='{}' > /dev/null 2>&1 &\"",
        safe_app, safe_dir
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
    claude_app_path: &Path,
) -> Result<(), String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Read system time: {e}"))?
        .as_nanos();
    let tmp_dir = env::temp_dir().join(format!("claude-multiprofile-{nanos}"));
    fs::create_dir_all(&tmp_dir).map_err(|e| format!("Create temp directory: {e}"))?;
    let script_path = tmp_dir.join("launcher.applescript");
    fs::write(&script_path, build_launch_applescript(data_dir, claude_app_path))
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
    copy_claude_icon(app_path, claude_app_path);
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
    compile_launcher_app(&clean_name, &data_dir, &app_path, &claude_app_path)?;

    registry.profiles.push(RegistryProfile {
        name: clean_name.clone(),
        profile_type: "desktop".to_string(),
        desktop: Some(RegistryDesktop {
            data_dir: data_dir.to_string_lossy().to_string(),
            app_path: app_path.to_string_lossy().to_string(),
            claude_app_path: claude_app_path.to_string_lossy().to_string(),
        }),
        code: None,
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
    open.args(["-n", "-a", &app, "--args", "--user-data-dir", &install.data_dir]);
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
    // The CLI persists Code profiles as { configDir, aliasName }. We tolerate
    // missing fields rather than refusing to load a malformed registry.
    let code = profile.code.as_ref()?;
    let config_dir = code
        .get("configDir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())?;
    let alias_name = code
        .get("aliasName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(CodeInstall {
        id: format!("profile:{}", profile.name),
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
    Ok(installs)
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
        projects.push(CodeProject {
            display_path: decode_project_dir_name(&id),
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
        serde_json::Value::String(s) => {
            if s.len() > 60 { format!("{}…", &s[..60]) } else { s.clone() }
        }
        _ => {
            let s = serde_json::to_string(v).unwrap_or_default();
            if s.len() > 60 { format!("{}…", &s[..60]) } else { s }
        }
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
fn compute_row_states(row: &mut LibraryRow, supports_symlink: bool) {
    let mut digest_counts: HashMap<String, usize> = HashMap::new();
    let mut link_counts: HashMap<String, usize> = HashMap::new();
    let mut present_total = 0_usize;

    for cell in &row.cells {
        if !cell.present {
            continue;
        }
        present_total += 1;
        if let Some(d) = &cell.digest {
            *digest_counts.entry(d.clone()).or_insert(0) += 1;
        }
        if let Some(d) = &cell.link_target_digest {
            *link_counts.entry(d.clone()).or_insert(0) += 1;
        }
    }

    for cell in &mut row.cells {
        if !cell.present {
            cell.state = "absent".into();
            continue;
        }
        if supports_symlink {
            if let Some(d) = &cell.link_target_digest {
                if link_counts.get(d).copied().unwrap_or(0) >= 2 {
                    cell.state = "shared".into();
                    continue;
                }
            }
            cell.state = "independent".into();
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

    let mut rows: Vec<LibraryRow> = ids
        .into_keys()
        .map(|id| {
            let cells = per_install
                .iter()
                .map(|(install, exts)| {
                    let entry = exts.iter().find(|e| e.id == id);
                    let dir = Path::new(&install.data_dir).join(EXT_DIR_NAME).join(&id);
                    let link_d = symlink_target_digest(&dir);
                    LibraryCell {
                        install_id: install.id.clone(),
                        install_name: install.name.clone(),
                        data_dir: install.data_dir.clone(),
                        kind: install.kind.clone(),
                        state: String::new(),
                        present: entry.is_some(),
                        detail: entry.map(|e| {
                            if e.has_settings {
                                "files+settings".into()
                            } else {
                                "files".into()
                            }
                        }),
                        digest: None,
                        link_target_digest: link_d,
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

    for row in &mut rows {
        compute_row_states(row, true);
    }
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
        compute_row_states(row, false);
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

    let mut rows: Vec<LibraryRow> = ids
        .into_iter()
        .map(|(id, (name, desc, creator))| {
            let cells = per_install
                .iter()
                .map(|(install, combo, manifest)| {
                    let entry_owned = manifest_skill_entries(manifest)
                        .into_iter()
                        .find(|e| entry_skill_id(e) == Some(&id))
                        .cloned();
                    let (present, detail, digest, link_d) = match entry_owned {
                        Some(entry) => {
                            let enabled = entry_enabled(&entry);
                            let detail = if enabled { "enabled" } else { "disabled" };
                            let d = value_digest(&entry);
                            let link_d = combo.as_ref().and_then(|c| {
                                symlink_target_digest(&c.join(SKILLS_SUBDIR).join(&id))
                            });
                            (true, Some(detail.into()), Some(d), link_d)
                        }
                        None => (false, None, None, None),
                    };
                    LibraryCell {
                        install_id: install.id.clone(),
                        install_name: install.name.clone(),
                        data_dir: install.data_dir.clone(),
                        kind: install.kind.clone(),
                        state: String::new(),
                        present,
                        detail,
                        digest,
                        link_target_digest: link_d,
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

    for row in &mut rows {
        compute_row_states(row, true);
    }
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
        compute_row_states(row, false);
    }
    Ok(rows)
}

// ----- Unified library apply -----

/// Dispatch a single cell flip to the right pair-wise apply helper.
/// Auto-picks a source: explicit `source_install_id` if provided, else the
/// first present cell in the same row that isn't the target.
pub fn apply_library_change(
    kind: String,
    change: LibraryCellChange,
) -> Result<bool, String> {
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
    let mut per_install: Vec<(DesktopInstall, Vec<LocalSession>, Option<String>)> =
        Vec::with_capacity(installs.len());

    for install in installs {
        let data_dir = PathBuf::from(&install.data_dir);
        let root = match kind {
            SessionKind::CodePanel => code_sessions_root(&data_dir),
            SessionKind::CoworkAgent => cowork_sessions_root(&data_dir),
        };
        let sessions = scan_sessions_under(&root);

        // Workspace symlink digest — shared with the existing code-history
        // sharing logic. Only meaningful for the code panel; cowork agent
        // mode is per-account, not per-workspace.
        let link_d = match kind {
            SessionKind::CodePanel => {
                let ws = read_workspace_identity(&data_dir).unwrap_or(None);
                ws.as_ref()
                    .map(|w| desktop_code_workspace_path(&data_dir, w))
                    .as_deref()
                    .and_then(symlink_target_digest)
            }
            SessionKind::CoworkAgent => None,
        };
        per_install.push((install, sessions, link_d));
    }

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
            .map(|(install, sessions, link_d)| {
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
                    state: String::new(),
                    present: n > 0,
                    detail,
                    digest: None,
                    link_target_digest: link_d.clone(),
                }
            })
            .collect();
        let mut summary = LibraryRow {
            id: "__workspace__".into(),
            label: "Whole workspace".into(),
            description: Some(
                "Toggle to symlink the entire `claude-code-sessions/` workspace between profiles.".into(),
            ),
            cells,
            interactive: true,
            group: Some("Workspace".into()),
        };
        compute_row_states(&mut summary, /* supports_symlink */ true);
        rows.push(summary);
    }

    // One row per cwd / processName.
    for key in keys {
        let cells: Vec<LibraryCell> = per_install
            .iter()
            .map(|(install, sessions, link_d)| {
                let matching: Vec<&LocalSession> = sessions
                    .iter()
                    .filter(|s| project_key(s, kind).as_deref() == Some(key.as_str()))
                    .collect();
                let matching_owned: Vec<LocalSession> =
                    matching.iter().map(|s| (*s).clone()).collect();
                build_session_cell(install, &matching_owned, link_d.clone())
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
        let mut row = LibraryRow {
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
        };
        compute_row_states(&mut row, /* supports_symlink */ true);
        rows.push(row);
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
            commands::list_library_preferences,
            commands::apply_library_changes,
            commands::list_library_code_history,
            commands::list_library_cowork_sessions,
            commands::list_sessions_for_project,
            commands::get_profile_stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running Claudius");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
    fn compute_row_states_symlink_groups_two_or_more_as_shared() {
        let mut row = LibraryRow {
            id: "ext-a".into(),
            label: "ext-a".into(),
            description: None,
            cells: vec![
                make_cell("a", true, None, Some("link-X")),
                make_cell("b", true, None, Some("link-X")),
                make_cell("c", true, None, Some("link-Y")),
                make_cell("d", false, None, None),
            ],
            interactive: true,
            group: None,
        };
        compute_row_states(&mut row, /* supports_symlink */ true);
        assert_eq!(row.cells[0].state, "shared");
        assert_eq!(row.cells[1].state, "shared");
        assert_eq!(row.cells[2].state, "independent"); // only one in its link group
        assert_eq!(row.cells[3].state, "absent");
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
        compute_row_states(&mut row, /* supports_symlink */ false);
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
        compute_row_states(&mut row, false);
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
        compute_row_states(&mut row, false);
        assert_eq!(row.cells[0].state, "copied");
        assert_eq!(row.cells[1].state, "copied");
        assert_eq!(row.cells[2].state, "absent");
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
}

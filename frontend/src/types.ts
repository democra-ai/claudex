// Mirrors the Serialize types in src-tauri/src/lib.rs.
// Field names use snake_case to match Rust's default Serde output.

// -----------------------------
// Desktop side (Claude.app)
// -----------------------------

export type DesktopInstall = {
  id: string;
  name: string;
  /** "default" for the original Claude install, "profile" for managed ones.
   *  Orthogonal to is_running — the user might use a profile as their
   *  primary even when "default" exists on disk. */
  kind: "default" | "profile";
  data_dir: string;
  app_path: string | null;
  launcher_path: string | null;
  managed: boolean;
  /** True when a Claude.app process is currently open against this data_dir.
   *  Detected by parsing --user-data-dir from running `ps` output. */
  is_running: boolean;
};

export type PairExtensionShare = {
  id: string;
  source_has_extension: boolean;
  target_has_extension: boolean;
  source_has_settings: boolean;
  target_has_settings: boolean;
  /** True iff the *folder + settings* are both linked between profiles. */
  shared: boolean;
  /** Folder linked but settings out of sync (or vice versa). */
  partial: boolean;
  direction: "source-to-target" | "target-to-source" | "independent";
};

export type PairShareChange = {
  extension_id: string;
  shared: boolean;
};

// -----------------------------
// Code side (Claude Code CLI)
// -----------------------------

export type CodeInstall = {
  id: string;
  name: string;
  /** "default" for the implicit ~/.claude install, "profile" for managed ones. */
  kind: "default" | "profile";
  config_dir: string;
  alias_name: string | null;
  managed: boolean;
};

export type CodeProject = {
  id: string;
  display_path: string;
  session_count: number;
  total_bytes: number;
  last_modified_ms: number;
  first_message_preview: string | null;
};

export type PairCodeProjectShare = {
  id: string;
  display_path: string;
  source_present: boolean;
  target_present: boolean;
  source_session_count: number;
  target_session_count: number;
  source_bytes: number;
  target_bytes: number;
  source_last_modified_ms: number;
  target_last_modified_ms: number;
  shared: boolean;
  direction: "source-to-target" | "target-to-source" | "independent";
};

export type PairCodeShareChange = {
  project_id: string;
  shared: boolean;
};

// -----------------------------
// Desktop-embedded Claude Code history (claude-code-sessions/)
// -----------------------------

export type DesktopCodeWorkspaceRef = {
  device_id: string;
  workspace_id: string;
};

export type DesktopCodeHistoryStat = {
  present: boolean;
  session_count: number;
  total_bytes: number;
  last_activity_ms: number;
  /** Up to 5 cwds, ordered by most-recent activity. */
  recent_cwds: string[];
  /**
   * The most-recently-active <deviceId>/<workspaceId>/ tuple, if any.
   * If null, the profile has no Code workspace yet — Desktop must launch
   * the Code panel once for it to be created.
   */
  primary_workspace: DesktopCodeWorkspaceRef | null;
};

export type PairDesktopCodeHistory = {
  source: DesktopCodeHistoryStat;
  target: DesktopCodeHistoryStat;
  /** True iff target's primary workspace is a live symlink at source's. */
  shared: boolean;
  direction: "source-to-target" | "target-to-source" | "independent";
  /** True iff target has no <dev>/<ws>/ workspace yet. */
  target_needs_bootstrap: boolean;
  /** Same for source. */
  source_needs_bootstrap: boolean;
  /**
   * True iff a previous version of the app left a whole-`claude-code-sessions/`
   * symlink in place. Toggling will clean it up first.
   */
  legacy_whole_dir_link: boolean;
};

export type PairDesktopCodeHistoryChange = {
  shared: boolean;
};

// -----------------------------
// Desktop MCP servers (Model B — copy on apply)
// -----------------------------

export type PairMcpServerShare = {
  name: string;
  source_present: boolean;
  target_present: boolean;
  /** Short human label, e.g. "npx @modelcontextprotocol/server-..." */
  source_summary: string | null;
  target_summary: string | null;
  /** True iff both sides define this server with an identical config value. */
  copied: boolean;
};

export type PairMcpServerChange = {
  name: string;
  /** New desired state: true = copy from source, false = remove from target. */
  copied: boolean;
};

// -----------------------------
// Desktop Cowork Skills (Model A — symlink + manifest patch)
// -----------------------------

export type PairCoworkSkillShare = {
  skill_id: string;
  name: string;
  description: string | null;
  source_present: boolean;
  target_present: boolean;
  source_enabled: boolean;
  target_enabled: boolean;
  shared: boolean;
};

export type PairCoworkSkillsResult = {
  rows: PairCoworkSkillShare[];
  /** True iff source has no <dev>/<acct>/ skills-plugin combo yet — open the
   * Cowork panel in that profile once so the folder is created. */
  source_needs_bootstrap: boolean;
  target_needs_bootstrap: boolean;
};

export type PairCoworkSkillChange = {
  skill_id: string;
  shared: boolean;
};

// -----------------------------
// Desktop Preferences (Model B — copy on apply, allowlisted keys)
// -----------------------------

export type PreferenceScope = "ui" | "desktop_pref";

export type PairPreferenceShare = {
  key: string;
  scope: PreferenceScope;
  label: string;
  source_present: boolean;
  target_present: boolean;
  source_value: unknown;
  target_value: unknown;
  copied: boolean;
};

export type PairPreferenceChange = {
  key: string;
  scope: PreferenceScope;
  copied: boolean;
};

// -----------------------------
// Library / matrix view — one row per content item, one cell per profile
// -----------------------------

export type CellState =
  | "shared"      // ■ live symlink, ≥2 profiles in the same link group
  | "copied"      // ● one-shot copy, value matches another profile
  | "diverged"    // ◐ present in ≥2 profiles with different values
  | "independent" // ○ present here, not aligned with any other profile
  | "absent";     // · not in this profile

export type LibraryCell = {
  install_id: string;
  install_name: string;
  data_dir: string;
  /** "default" | "profile" */
  kind: string;
  state: CellState;
  present: boolean;
  detail: string | null;
  digest: string | null;
  link_target_digest: string | null;
};

export type LibraryRow = {
  id: string;
  label: string;
  description: string | null;
  cells: LibraryCell[];
  /** When false, clicking a cell shouldn't stage a toggle — browse-only.
   *  Used for per-cwd Code/Cowork rows where the share unit is the
   *  workspace, not the individual project. */
  interactive: boolean;
};

export type LocalSession = {
  session_id: string;
  title: string | null;
  cwd: string | null;
  process_name: string | null;
  model: string | null;
  created_at_ms: number;
  last_activity_ms: number;
  account_name: string | null;
  email_address: string | null;
};

export type LibraryCellChange = {
  row_id: string;
  target_install_id: string;
  wants: boolean;
  source_install_id?: string;
};

export type LibraryKind =
  | "code_history"
  | "cowork_sessions"
  | "extensions"
  | "mcp_servers"
  | "cowork_skills"
  | "preferences";

// -----------------------------
// Profile stats — codexbar-style status panel
// -----------------------------

export type ProfileIdentity = {
  account_id: string;
  /** True for the account that owns this Desktop profile (from
   *  cowork-enabled-cli-ops.json). */
  is_owner: boolean;
  account_name: string | null;
  email_address: string | null;
  agent_session_count: number;
  last_activity_ms: number | null;
};

export type ProfileStats = {
  install_id: string;
  install_name: string;
  kind: string;
  data_dir: string;
  account_id: string | null;
  org_id: string | null;
  /** All identities (Anthropic accounts) present in this profile. Owner
   *  first, then co-users by recency. */
  identities: ProfileIdentity[];
  /** Tokens consumed today across all accounts in this Desktop instance.
   *  Source: buddy-tokens.json. Resets daily by Claude Desktop. */
  tokens_today: number;
  tokens_today_date: string | null;
  /** Time-windowed code session counts — the codexbar-style "do I have
   *  headroom right now?" reading. */
  code_sessions_last_5h: number;
  code_sessions_last_24h: number;
  code_sessions_last_7d: number;
  code_sessions_last_30d: number;
  /** Avg sessions/day over prior 7 days (excluding today) — the pace
   *  baseline today is compared against. */
  code_sessions_per_day_baseline: number;
  code_sessions_today: number;
  /** Most-used model in the last 7 days, normalized (e.g. "opus-4-7"). */
  top_model_last_7d: string | null;
  /** Device id from ant-did, base64-decoded. Useful to spot machine drift. */
  device_id: string | null;
  ssh_remote_count: number;
  disk_bytes: number | null;
  code_panel_bytes: number | null;
  cowork_agent_bytes: number | null;
  created_at_ms: number | null;
  last_activity_ms: number | null;
  code_session_count: number;
  code_total_bytes: number;
  code_recent_cwds: string[];
  cowork_session_count: number;
  extension_count: number;
  mcp_server_count: number;
  cowork_skill_count: number;
  /** 8-hex prefix of the workspace link target; same digest = same group. */
  link_group: string | null;
  /** install_ids that share this workspace via symlink. */
  shared_with: string[];
};

// -----------------------------
// Shared
// -----------------------------

export type CopySummary = {
  copied: number;
  skipped: number;
};

/** UI-only: which kind of content tab the user is on. */
export type ContentKind =
  // Desktop tabs
  | "extensions"
  | "mcp_servers"
  | "cowork_skills"
  | "preferences"
  | "code_history"
  // Code-CLI tabs
  | "history";

/**
 * Discriminated union the UI uses to treat desktop + code installs uniformly.
 * `category` is the toggle the rest of the app routes on.
 */
export type Profile =
  | ({ category: "desktop" } & DesktopInstall)
  | ({ category: "code" } & CodeInstall);

/**
 * Globally-unique key across categories. We prepend the category because
 * "default" exists in both desktop and code worlds.
 */
export function profileKey(profile: Profile): string {
  return `${profile.category}:${profile.id}`;
}

/** Path the backend uses for content listing — different field per category. */
export function profileRootPath(profile: Profile): string {
  return profile.category === "desktop" ? profile.data_dir : profile.config_dir;
}

/** Display label, accounting for the "default" alias on each side. */
export function profileLabel(profile: Profile): string {
  if (profile.kind === "default") {
    return profile.category === "desktop" ? "Default Desktop" : "Default ~/.claude";
  }
  return profile.name;
}

/** A row in the multi-select share table. Generic enough for any content kind. */
export type ShareRow = {
  id: string;
  /** Human-readable label; defaults to id. */
  label?: string;
  source_present: boolean;
  target_present: boolean;
  shared: boolean;
  partial: boolean;
  /** Optional secondary line (e.g. "Files + Settings"). */
  source_detail?: string;
  target_detail?: string;
};

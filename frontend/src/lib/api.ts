import { invoke } from "@tauri-apps/api/core";
import type {
  CodeInstall,
  CodexInstall,
  CodeProject,
  CopySummary,
  DesktopInstall,
  LibraryCellChange,
  LibraryKind,
  LibraryRow,
  LocalSession,
  ProfileStats,
  PairCodeProjectShare,
  PairCodeShareChange,
  PairCoworkSkillChange,
  PairCoworkSkillsResult,
  PairDesktopCodeHistory,
  PairDesktopCodeHistoryChange,
  PairExtensionShare,
  PairMcpServerChange,
  PairMcpServerShare,
  PairPreferenceChange,
  PairPreferenceShare,
  PairShareChange,
} from "@/types";

/**
 * Type-safe Tauri command wrapper. Every function here corresponds to a
 * #[tauri::command] in src-tauri/src/lib.rs (commands module). Names and
 * arg shapes must stay in sync — Tauri auto-converts camelCase JS args to
 * snake_case Rust parameters.
 */
export const api = {
  // -------- Desktop profiles --------
  listDesktopInstalls(): Promise<DesktopInstall[]> {
    return invoke("list_desktop_installs");
  },

  createDesktopProfile(name: string): Promise<DesktopInstall> {
    return invoke("create_desktop_profile", { name });
  },

  createCodeProfile(name: string, seedFromDefault: boolean): Promise<CodeInstall> {
    return invoke("create_code_profile", { name, seedFromDefault });
  },

  launchDesktopInstall(installId: string): Promise<void> {
    return invoke("launch_desktop_install", { installId });
  },

  // -------- Desktop extension sharing --------
  listPairSharing(
    sourceDataDir: string,
    targetDataDir: string,
  ): Promise<PairExtensionShare[]> {
    return invoke("list_pair_sharing", { sourceDataDir, targetDataDir });
  },

  applyPairSharing(
    sourceDataDir: string,
    targetDataDir: string,
    changes: PairShareChange[],
  ): Promise<CopySummary> {
    return invoke("apply_pair_sharing", {
      sourceDataDir,
      targetDataDir,
      changes,
    });
  },

  // -------- Code profiles --------
  listCodeInstalls(): Promise<CodeInstall[]> {
    return invoke("list_code_installs");
  },

  listCodexInstalls(): Promise<CodexInstall[]> {
    return invoke("list_codex_installs");
  },

  createCodexProfile(name: string): Promise<CodexInstall> {
    return invoke("create_codex_profile", { name });
  },

  launchCodexInstall(installId: string): Promise<void> {
    return invoke("launch_codex_install", { installId });
  },

  listCodeHistory(configDir: string): Promise<CodeProject[]> {
    return invoke("list_code_history", { configDir });
  },

  // -------- Code history sharing --------
  listPairCodeHistorySharing(
    sourceConfigDir: string,
    targetConfigDir: string,
  ): Promise<PairCodeProjectShare[]> {
    return invoke("list_pair_code_history_sharing", {
      sourceConfigDir,
      targetConfigDir,
    });
  },

  applyPairCodeHistorySharing(
    sourceConfigDir: string,
    targetConfigDir: string,
    changes: PairCodeShareChange[],
  ): Promise<CopySummary> {
    return invoke("apply_pair_code_history_sharing", {
      sourceConfigDir,
      targetConfigDir,
      changes,
    });
  },

  // -------- Desktop-embedded Claude Code history sharing --------
  // Targets <dataDir>/claude-code-sessions/, the chat panel inside Claude Desktop.
  listPairDesktopCodeHistory(
    sourceDataDir: string,
    targetDataDir: string,
  ): Promise<PairDesktopCodeHistory> {
    return invoke("list_pair_desktop_code_history", {
      sourceDataDir,
      targetDataDir,
    });
  },

  applyPairDesktopCodeHistory(
    sourceDataDir: string,
    targetDataDir: string,
    change: PairDesktopCodeHistoryChange,
  ): Promise<CopySummary> {
    return invoke("apply_pair_desktop_code_history", {
      sourceDataDir,
      targetDataDir,
      change,
    });
  },

  // -------- Desktop MCP server sharing --------
  // Targets `mcpServers` in <dataDir>/claude_desktop_config.json.
  listPairMcpSharing(
    sourceDataDir: string,
    targetDataDir: string,
  ): Promise<PairMcpServerShare[]> {
    return invoke("list_pair_mcp_sharing", { sourceDataDir, targetDataDir });
  },

  applyPairMcpSharing(
    sourceDataDir: string,
    targetDataDir: string,
    changes: PairMcpServerChange[],
  ): Promise<CopySummary> {
    return invoke("apply_pair_mcp_sharing", {
      sourceDataDir,
      targetDataDir,
      changes,
    });
  },

  // -------- Desktop Cowork Skills sharing --------
  // Targets <dataDir>/local-agent-mode-sessions/skills-plugin/<dev>/<acct>/.
  listPairCoworkSkillsSharing(
    sourceDataDir: string,
    targetDataDir: string,
  ): Promise<PairCoworkSkillsResult> {
    return invoke("list_pair_cowork_skills_sharing", {
      sourceDataDir,
      targetDataDir,
    });
  },

  applyPairCoworkSkillsSharing(
    sourceDataDir: string,
    targetDataDir: string,
    changes: PairCoworkSkillChange[],
  ): Promise<CopySummary> {
    return invoke("apply_pair_cowork_skills_sharing", {
      sourceDataDir,
      targetDataDir,
      changes,
    });
  },

  // -------- Desktop Preferences sharing --------
  // Targets allowlisted keys in <dataDir>/config.json and
  // <dataDir>/claude_desktop_config.json → preferences.
  listPairPreferenceSharing(
    sourceDataDir: string,
    targetDataDir: string,
  ): Promise<PairPreferenceShare[]> {
    return invoke("list_pair_preference_sharing", {
      sourceDataDir,
      targetDataDir,
    });
  },

  applyPairPreferenceSharing(
    sourceDataDir: string,
    targetDataDir: string,
    changes: PairPreferenceChange[],
  ): Promise<CopySummary> {
    return invoke("apply_pair_preference_sharing", {
      sourceDataDir,
      targetDataDir,
      changes,
    });
  },

  // -------- Library / matrix view --------
  // One call returns the full (item × profile) grid for the kind.
  listLibrary(kind: LibraryKind): Promise<LibraryRow[]> {
    const command =
      kind === "code_history"
        ? "list_library_code_history"
        : kind === "cowork_sessions"
        ? "list_library_cowork_sessions"
        : kind === "extensions"
        ? "list_library_extensions"
        : kind === "mcp_servers"
        ? "list_library_mcp"
        : kind === "cowork_skills"
        ? "list_library_cowork_skills"
        : "list_library_preferences";
    return invoke(command);
  },

  listSessionsForProject(
    installId: string,
    rowId: string,
    isCowork: boolean,
  ): Promise<LocalSession[]> {
    return invoke("list_sessions_for_project", {
      installId,
      rowId,
      isCowork,
    });
  },

  applyLibraryChanges(
    kind: LibraryKind,
    changes: LibraryCellChange[],
  ): Promise<CopySummary> {
    return invoke("apply_library_changes", { kind, changes });
  },

  getProfileStats(installId: string): Promise<ProfileStats> {
    return invoke("get_profile_stats", { installId });
  },
};

/** True when running inside the Tauri shell, false in a plain `vite preview`. */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

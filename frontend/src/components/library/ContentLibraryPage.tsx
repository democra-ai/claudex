import { useCallback, useEffect, useMemo, useState } from "react";
import { Info, Play, Plus, Share2, X } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { api, isTauri } from "@/lib/api";
import { useToasts } from "@/hooks/useToast";
import type {
  CodeInstall,
  CodexInstall,
  DesktopInstall,
  LibraryCellChange,
  LibraryKind,
  LibraryRow,
} from "@/types";
import { KindNav, computeKindCount } from "./KindNav";
import { Matrix } from "./Matrix";
import { pendingKeyFor, type PendingChange } from "./pending";
import { DetailSheet, type Selection } from "./DetailSheet";
import { PendingBar } from "./PendingBar";
import { DeleteProfileButton } from "./DeleteProfileButton";
import { ClaudeMark, CodexMark } from "./PlatformMarks";

/**
 * A walled-off profile world (Claude / Codex). Each region owns a tinted
 * sticky header with its accent swatch, a count, and its own "+" that adds
 * a profile of THAT type — plus a one-line scope caption. The visual wall
 * (accent tint + heavier divider between regions) is how we communicate that
 * Claude and Codex share independently.
 */
function SidebarRegion({
  label,
  accent,
  caption,
  count,
  adding,
  onAddToggle,
  children,
}: {
  label: string;
  accent: "claude" | "codex";
  caption: string;
  count: string;
  adding: boolean;
  onAddToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div
        className={cn(
          "flex items-center justify-between rounded-md px-3 py-1.5",
          accent === "claude" ? "bg-primary/5" : "bg-foreground/5",
        )}
      >
        <div className="flex items-center gap-2">
          <span
            className={cn(
              "h-2.5 w-2.5 rounded-[3px]",
              accent === "claude" ? "bg-primary" : "bg-foreground",
            )}
          />
          <span className="font-sans text-[10px] uppercase tracking-[0.14em] text-muted-foreground/80">
            {label}
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <span className="font-mono text-[10px] tabular-nums text-muted-foreground/60">
            {count}
          </span>
          <Button
            type="button"
            size="icon"
            variant="ghost"
            onClick={onAddToggle}
            className="h-5 w-5 rounded"
            aria-label={`Add ${label} profile`}
            aria-expanded={adding}
          >
            {adding ? <X className="h-3 w-3" /> : <Plus className="h-3.5 w-3.5" />}
          </Button>
        </div>
      </div>
      <p className="px-3 pb-1 pt-0.5 font-sans text-[10px] leading-snug text-muted-foreground/60">
        {caption}
      </p>
      <div className="space-y-0.5 px-1">{children}</div>
    </div>
  );
}

/** Inline name entry that slides into a region body when its "+" is clicked.
 *  Enter confirms, Escape cancels. */
function AddProfileInput({
  value,
  onChange,
  onConfirm,
  onCancel,
  busy,
  hint,
}: {
  value: string;
  onChange: (v: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
  busy: boolean;
  hint: string;
}) {
  return (
    <div className="mb-1 px-1">
      <div className="flex gap-1">
        <Input
          autoFocus
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && value.trim()) onConfirm();
            if (e.key === "Escape") onCancel();
          }}
          placeholder="profile name"
          className="h-7 font-sans text-xs"
          disabled={busy}
        />
        <Button
          type="button"
          size="icon"
          onClick={onConfirm}
          disabled={busy || !value.trim()}
          className="h-7 w-7"
          aria-label="Create profile"
        >
          <Plus className="h-3.5 w-3.5" />
        </Button>
      </div>
      <p className="px-1 pt-1 font-sans text-[10px] leading-snug text-muted-foreground/70">
        {hint}
      </p>
    </div>
  );
}

const EMPTY_HINTS: Record<LibraryKind, string> = {
  code_history: "No Cowork code sessions yet in any profile.",
  cowork_sessions: "No Cowork agent-mode sessions in any profile.",
  extensions: "No extensions installed in any profile.",
  mcp_servers: "No MCPs configured in any claude_desktop_config.json.",
  cowork_skills: "No Cowork skills — open Cowork in any profile once.",
  skills: "No skills found in ~/.claude/skills or ~/.codex/skills.",
  preferences: "Allowlisted preferences not set in any profile.",
  codex_sessions: "No Codex sessions in ~/.codex/sessions yet.",
  codex_skills: "No skills in ~/.codex/skills yet.",
  codex_mcp: "No MCP servers in ~/.codex/config.toml.",
  mcp_cross: "No stdio MCP servers found in Claude Code or Codex.",
  memory: "No CLAUDE.md in any Claude account yet.",
  codex_memory: "No AGENTS.md in any Codex account yet.",
  memory_cross: "No CLAUDE.md / AGENTS.md memory file in any account yet.",
};

/** Per-kind one-liner shown above the matrix explaining the Codex situation. */
const KIND_SCOPE_CAPTION: Partial<Record<LibraryKind, string>> = {
  skills: "Skills are the one library Claude and Codex share — the Codex column links into ~/.codex/skills.",
  mcp_cross:
    "Copy stdio MCP servers between Claude Code (~/.claude.json, JSON) and Codex (~/.codex/config.toml, TOML). Toggle a cell to copy from the other side; collisions with a different config are refused.",
  memory_cross:
    "Share the agent memory file (Claude CLAUDE.md ↔ Codex AGENTS.md) between any accounts — same platform or across. It's just Markdown, so it links live like a skill. Toggle a cell to link it to another account's memory.",
  memory:
    "Share CLAUDE.md (agent memory) between your Claude accounts — a live symlink. Toggle a cell to link it to another account's memory.",
  codex_memory:
    "Share AGENTS.md (agent memory) between your Codex accounts — a live symlink. Toggle a cell to link it to another account's memory.",
  extensions: "Claude-only — Codex has no extensions equivalent.",
  mcp_servers: "Claude uses JSON, Codex uses TOML — cross-tool MCP would be copy-with-transform, not yet available.",
  cowork_skills: "Claude Cowork only — these are the per-profile Cowork agent skills.",
  cowork_sessions: "Claude-only content.",
  preferences: "Claude-only content.",
  codex_sessions:
    "Codex sessions grouped by project, one column per account. Open a project to import a session into Claude Code; toggle 'All Codex sessions' to symlink-share the whole sessions dir between accounts.",
  code_history:
    "Claude Code sessions by project. Open a project to export a session into Codex.",
  codex_skills:
    "Skills per Codex account (~/.codex-<name>/skills). Toggle a cell to share a skill between accounts (live symlink).",
  codex_mcp:
    "MCP servers per Codex account (config.toml). Toggle a cell to copy a server between accounts.",
};

/** Synthetic DesktopInstall-shaped column for the global Codex library. Codex
 *  agent state (sessions / skills / config.toml) lives at ~/.codex and is shared
 *  by every Codex profile (the launchers isolate only the Chromium login), so a
 *  single column represents all of it. */
const CODEX_GLOBAL_COLUMN: DesktopInstall = {
  id: "codex:global",
  name: "Codex",
  kind: "profile",
  data_dir: "~/.codex",
  app_path: null,
  launcher_path: null,
  managed: true,
  is_running: false,
};

/** Synthetic column for the Claude Code user-scope MCP config (~/.claude.json),
 *  used as the Claude side of the cross-tool MCP matrix. */
const CLAUDE_CODE_MCP_COLUMN: DesktopInstall = {
  id: "claude:code",
  name: "Claude Code",
  kind: "default",
  data_dir: "~/.claude.json",
  app_path: null,
  launcher_path: null,
  managed: true,
  is_running: false,
};

/** The three Codex-private content kinds (global to ~/.codex). */
const CODEX_KINDS: LibraryKind[] = [
  "codex_sessions",
  "codex_skills",
  "codex_mcp",
  "codex_memory",
];

interface SidebarProfileRowProps {
  profile: DesktopInstall;
  visible: boolean;
  selected: boolean;
  onToggleVisible: () => void;
  onSelect: () => void;
  onLaunch: () => void;
  onDelete: (deleteData: boolean) => Promise<void>;
  busy: boolean;
}

function SidebarProfileRow({
  profile,
  visible,
  selected,
  onToggleVisible,
  onSelect,
  onLaunch,
  onDelete,
  busy,
}: SidebarProfileRowProps) {
  const running = profile.is_running;
  return (
    <div
      className={cn(
        "group flex items-center gap-1.5 rounded-md pl-1.5 pr-1 transition-colors",
        selected ? "bg-primary/8" : "hover:bg-muted/60",
        running && !selected && "bg-primary/4",
        !visible && "opacity-55",
      )}
    >
      <Checkbox
        checked={visible}
        onCheckedChange={onToggleVisible}
        aria-label={`Show ${profile.name} column`}
        className="h-3.5 w-3.5"
      />
      <button
        type="button"
        onClick={onSelect}
        className={cn(
          "flex min-w-0 flex-1 items-center gap-2 py-1.5 pl-1 pr-1 text-left",
        )}
        title={
          running
            ? `${profile.name} — currently running`
            : `Show ${profile.name} details`
        }
      >
        {/* Status dot: pulsing green when live, muted otherwise. */}
        <span className="relative inline-flex h-2 w-2 shrink-0 items-center justify-center">
          {running ? (
            <>
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-primary/60" />
              <span className="relative inline-flex h-2 w-2 rounded-full bg-primary" />
            </>
          ) : (
            <span
              className={cn(
                "inline-block h-1.5 w-1.5 rounded-full",
                profile.kind === "default"
                  ? "bg-muted-foreground/60"
                  : "bg-muted-foreground/30",
              )}
            />
          )}
        </span>
        <span
          className={cn(
            "truncate font-sans text-[13px]",
            running && "font-medium",
          )}
        >
          {profile.kind === "default" ? "Default" : profile.name}
        </span>
        {running ? (
          <span className="ml-auto rounded-full bg-primary/15 px-1.5 py-0.5 font-sans text-[9px] uppercase tracking-wider text-primary">
            live
          </span>
        ) : selected ? (
          <Info className="ml-auto h-3 w-3 shrink-0 text-primary" />
        ) : null}
      </button>
      <button
        type="button"
        onClick={onLaunch}
        disabled={busy || running}
        className="shrink-0 rounded-md p-1 text-muted-foreground opacity-0 transition-all hover:bg-primary/10 hover:text-primary group-hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-40"
        title={running ? `${profile.name} is already running` : `Launch ${profile.name}`}
        aria-label={running ? `${profile.name} is already running` : `Launch ${profile.name}`}
      >
        <Play className="h-3 w-3" />
      </button>
      <DeleteProfileButton
        name={profile.name}
        kind={profile.kind}
        isRunning={running}
        world="claude"
        busy={busy}
        onDelete={onDelete}
      />
    </div>
  );
}

type WorkTab = "claude" | "codex" | "share";

/** The three top-level worlds. Claude + Codex manage their own profiles and
 *  private content; Share is the cross-tool grid. Mirrors the segmented
 *  control at the top of Claude Desktop's own sidebar. */
function SegmentedTabs({
  value,
  onChange,
}: {
  value: WorkTab;
  onChange: (t: WorkTab) => void;
}) {
  const tabs: { id: WorkTab; label: string }[] = [
    { id: "claude", label: "Claude" },
    { id: "codex", label: "Codex" },
    { id: "share", label: "Share" },
  ];
  const icon = (id: WorkTab, active: boolean) => {
    if (id === "claude")
      return <ClaudeMark className={cn("h-3.5 w-3.5", active ? "text-primary" : "")} />;
    if (id === "codex")
      return (
        <CodexMark className={cn("h-3 w-3", active ? "text-[#4366F2]" : "")} />
      );
    return <Share2 className="h-3 w-3" />;
  };
  return (
    <div className="grid grid-cols-3 gap-0.5 rounded-lg bg-muted/60 p-0.5">
      {tabs.map((t) => {
        const active = value === t.id;
        return (
          <button
            key={t.id}
            type="button"
            onClick={() => onChange(t.id)}
            className={cn(
              "flex items-center justify-center gap-1.5 rounded-md py-1.5 font-sans text-[12px] transition-colors",
              active
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {icon(t.id, active)}
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

const CLAUDE_KINDS: LibraryKind[] = [
  "code_history",
  "cowork_sessions",
  "extensions",
  "mcp_servers",
  "cowork_skills",
  "memory",
  "preferences",
];

export default function ContentLibraryPage() {
  const [installs, setInstalls] = useState<DesktopInstall[]>([]);
  const [codexInstalls, setCodexInstalls] = useState<CodexInstall[]>([]);
  const [codeInstalls, setCodeInstalls] = useState<CodeInstall[]>([]);
  const [visibleIds, setVisibleIds] = useState<Set<string>>(new Set());
  const [activeTab, setActiveTab] = useState<WorkTab>("claude");
  const [activeKind, setActiveKind] = useState<LibraryKind>("code_history");
  const [rowsByKind, setRowsByKind] = useState<
    Partial<Record<LibraryKind, LibraryRow[]>>
  >({});
  // Pending toggles. The Map VALUE carries the structured (rowId, installId,
  // wants) so we never parse an ambiguous colon-joined key — install ids are
  // namespaced (claude-code:default, profile:<name>, codex:profile:<name>) and
  // some row ids contain colons (preferences "scope:key"), so any string split
  // would corrupt them. The key uses a  delimiter that can't collide.
  const [pending, setPending] = useState<Map<string, PendingChange>>(new Map());
  const [selection, setSelection] = useState<Selection>(null);
  const [busy, setBusy] = useState(false);
  const [applying, setApplying] = useState(false);
  const [importing, setImporting] = useState(false);
  const [loadingKind, setLoadingKind] = useState<LibraryKind | null>(null);
  // Per-region inline add state — each region's "+" toggles its own input.
  const [claudeAdding, setClaudeAdding] = useState(false);
  const [codexAdding, setCodexAdding] = useState(false);
  const [claudeName, setClaudeName] = useState("");
  const [codexName, setCodexName] = useState("");
  const { toasts, push, dismiss } = useToasts();

  // Display order: live profile first (so the user sees their *current*
  // working set front-and-center, not whatever happens to be on disk first),
  // then the un-renamed Default install, then managed profiles alpha.
  // This addresses a subtle but important UX bug: kind === "default" only
  // means "the install at the canonical path," NOT "currently in use" —
  // a user might do all their work in a renamed profile.
  const sortedInstalls = useMemo(() => {
    const score = (i: DesktopInstall) => {
      if (i.is_running) return 0;
      if (i.kind === "default") return 1;
      return 2;
    };
    return [...installs].sort((a, b) => {
      const sa = score(a);
      const sb = score(b);
      if (sa !== sb) return sa - sb;
      return a.name.localeCompare(b.name);
    });
  }, [installs]);

  const visibleProfiles = useMemo(
    () => sortedInstalls.filter((i) => visibleIds.has(i.id)),
    [sortedInstalls, visibleIds],
  );

  // The Skills kind is cross-tool: its columns are the Claude CODE config dirs
  // (~/.claude, ~/.claude-<name>) plus ONE global Codex column — not the
  // Desktop-profile columns the other kinds use. We feed the Matrix synthetic
  // DesktopInstall-shaped columns whose ids match the backend cell ids.
  const matrixColumns = useMemo<DesktopInstall[]>(() => {
    // Codex-private kinds: one column per Codex profile (each its own
    // ~/.codex-<name>). Cells line up by CodexInstall id ("default" /
    // "profile:<name>").
    if (CODEX_KINDS.includes(activeKind)) {
      const cols: DesktopInstall[] = codexInstalls.map((c) => ({
        id: c.id,
        name: c.kind === "default" ? "Default" : c.name,
        kind: c.kind,
        data_dir: c.data_dir,
        app_path: null,
        launcher_path: null,
        managed: c.managed,
        is_running: c.is_running,
      }));
      // The backend ALWAYS emits a "default" (~/.codex) column, but
      // list_codex_installs omits the default install when Codex.app / its
      // Desktop data dir is absent (CLI-only machines). Without a matching
      // column those default cells — real ~/.codex content — would be silently
      // dropped, so guarantee a "default" column here.
      if (!cols.some((c) => c.id === "default")) {
        cols.unshift({
          id: "default",
          name: "Default",
          kind: "default",
          data_dir: "~/.codex",
          app_path: null,
          launcher_path: null,
          managed: false,
          is_running: false,
        });
      }
      return cols;
    }
    // Cross-tool MCP: Claude Code + Codex, copy-mode.
    if (activeKind === "mcp_cross") return [CLAUDE_CODE_MCP_COLUMN, CODEX_GLOBAL_COLUMN];
    // Memory: every account is a column — Claude code dirs (CLAUDE.md) +
    // Codex homes (AGENTS.md). Ids namespaced to match the backend cells.
    if (activeKind === "memory_cross") {
      const claudeCols: DesktopInstall[] = codeInstalls.map((c) => ({
        id: `claude-code:${c.id}`,
        name: c.kind === "default" ? "Default ~/.claude" : c.name,
        kind: "default",
        data_dir: c.config_dir,
        app_path: null,
        launcher_path: null,
        managed: c.managed,
        is_running: false,
      }));
      const codexSrc =
        codexInstalls.length > 0
          ? codexInstalls.map((c) => ({ id: c.id, name: c.kind === "default" ? "Default" : c.name }))
          : [{ id: "default", name: "Default" }];
      const codexCols: DesktopInstall[] = codexSrc.map((c) => ({
        id: `codex:${c.id}`,
        name: `Codex ${c.name}`,
        kind: "profile",
        data_dir: "",
        app_path: null,
        launcher_path: null,
        managed: true,
        is_running: false,
      }));
      return [...claudeCols, ...codexCols];
    }
    // Within-Claude memory (CLAUDE.md): one column per Claude code install (plain ids).
    if (activeKind === "memory") {
      return codeInstalls.map((c) => ({
        id: c.id,
        name: c.kind === "default" ? "Default ~/.claude" : c.name,
        kind: c.kind,
        data_dir: c.config_dir,
        app_path: null,
        launcher_path: null,
        managed: c.managed,
        is_running: false,
      }));
    }
    // Within-Codex memory (AGENTS.md): one column per Codex home (plain ids).
    if (activeKind === "codex_memory") {
      const src =
        codexInstalls.length > 0
          ? codexInstalls
          : ([{ id: "default", name: "default", kind: "default", data_dir: "~/.codex", managed: false, is_running: false }] as CodexInstall[]);
      return src.map((c) => ({
        id: c.id,
        name: c.kind === "default" ? "Default" : c.name,
        kind: c.kind,
        data_dir: c.data_dir,
        app_path: null,
        launcher_path: null,
        managed: c.managed,
        is_running: c.is_running,
      }));
    }
    if (activeKind !== "skills") return visibleProfiles;
    // Cross-tool Skills: Claude CODE config dirs + one global Codex column.
    const claudeCols: DesktopInstall[] = codeInstalls.map((c) => ({
      id: c.id,
      name: c.name,
      kind: c.kind,
      data_dir: c.config_dir,
      app_path: null,
      launcher_path: null,
      managed: c.managed,
      is_running: false,
    }));
    return [...claudeCols, CODEX_GLOBAL_COLUMN];
  }, [activeKind, visibleProfiles, codeInstalls, codexInstalls]);

  const counts = useMemo(() => {
    const out: Partial<
      Record<LibraryKind, { synced: number; total: number } | null>
    > = {};
    for (const kind of [
      "code_history",
      "cowork_sessions",
      "extensions",
      "mcp_servers",
      "cowork_skills",
      "skills",
      "preferences",
      "codex_sessions",
      "codex_skills",
      "codex_mcp",
      "mcp_cross",
      "memory",
      "codex_memory",
      "memory_cross",
    ] as LibraryKind[]) {
      const rows = rowsByKind[kind];
      out[kind] = rows ? computeKindCount(rows) : null;
    }
    return out;
  }, [rowsByKind]);

  const resolveInstallName = useCallback(
    (installId: string) =>
      installs.find((i) => i.id === installId)?.name,
    [installs],
  );

  const loadInstalls = useCallback(async () => {
    if (!isTauri()) {
      push("Open via the Tauri shell to manage real profiles.", "info");
      return;
    }
    setBusy(true);
    try {
      // Codex + Code installs are independent dimensions — load them alongside
      // but don't fail the whole refresh if either isn't present.
      api.listCodexInstalls().then(setCodexInstalls).catch(() => setCodexInstalls([]));
      api.listCodeInstalls().then(setCodeInstalls).catch(() => setCodeInstalls([]));
      const list = await api.listDesktopInstalls();
      setInstalls(list);
      setVisibleIds((current) => {
        if (current.size === 0) return new Set(list.map((p) => p.id));
        const valid = new Set<string>();
        for (const p of list) if (current.has(p.id)) valid.add(p.id);
        return valid.size === 0 ? new Set(list.map((p) => p.id)) : valid;
      });
    } catch (e) {
      push(String(e), "error");
    } finally {
      setBusy(false);
    }
  }, [push]);

  const loadKind = useCallback(
    async (kind: LibraryKind) => {
      if (!isTauri()) return;
      setLoadingKind(kind);
      try {
        const rows = await api.listLibrary(kind);
        setRowsByKind((current) => ({ ...current, [kind]: rows }));
      } catch (e) {
        push(String(e), "error");
      } finally {
        setLoadingKind(null);
      }
    },
    [push],
  );

  useEffect(() => {
    loadInstalls();
  }, [loadInstalls]);

  // Poll running status every 10s so the "live" badge tracks reality
  // when the user opens/closes Claude.app in another window. The poll
  // only re-reads the install registry + ps, no heavy session scans.
  useEffect(() => {
    if (!isTauri()) return;
    const id = setInterval(() => {
      api
        .listDesktopInstalls()
        .then((list) => {
          // Only update if the running-state diff actually changed —
          // avoids unnecessary re-renders.
          setInstalls((current) => {
            if (current.length !== list.length) return list;
            const sameRunning = current.every((c, i) => c.is_running === list[i]?.is_running);
            return sameRunning ? current : list;
          });
        })
        .catch(() => undefined);
    }, 10_000);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    loadKind(activeKind);
  }, [activeKind, loadKind, installs.length]);

  useEffect(() => {
    if (installs.length === 0) return;
    loadKind(activeKind);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [installs.length]);

  // Eagerly load counts for the other kinds in the background so KindNav
  // shows N/M for everything, not just the active tab.
  useEffect(() => {
    const others: LibraryKind[] = [
      "code_history",
      "cowork_sessions",
      "extensions",
      "mcp_servers",
      "cowork_skills",
      "skills",
      "preferences",
      "codex_sessions",
      "codex_skills",
      "codex_mcp",
      "mcp_cross",
      "memory",
      "codex_memory",
      "memory_cross",
    ];
    const todo = others.filter((k) => k !== activeKind && !rowsByKind[k]);
    if (todo.length === 0 || !isTauri()) return;
    let cancelled = false;
    (async () => {
      for (const kind of todo) {
        if (cancelled) return;
        try {
          const rows = await api.listLibrary(kind);
          if (!cancelled) {
            setRowsByKind((current) => ({ ...current, [kind]: rows }));
          }
        } catch {
          /* count badge will stay blank — non-fatal */
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeKind, installs.length]);

  const handleCellToggle = useCallback(
    (rowId: string, installId: string, nextPresent: boolean) => {
      const rows = rowsByKind[activeKind];
      const row = rows?.find((r) => r.id === rowId);
      const cell = row?.cells.find((c) => c.install_id === installId);
      if (!cell) return;
      const key = pendingKeyFor(rowId, installId);
      setPending((current) => {
        const next = new Map(current);
        // For symlink content, "currently shared" is what we should compare —
        // for code-history the "present" check is too lax. Use the cell's
        // current effective state instead.
        const currentShared =
          cell.state === "shared" || cell.state === "copied";
        // Toggling back to the original state drops the pending entry.
        const wantsShared = nextPresent;
        if (wantsShared === currentShared && cell.present === wantsShared) {
          next.delete(key);
        } else {
          next.set(key, { rowId, installId, wants: wantsShared });
        }
        return next;
      });
    },
    [rowsByKind, activeKind],
  );

  const handleApply = useCallback(async () => {
    if (pending.size === 0) return;
    const changes: LibraryCellChange[] = [];
    for (const { rowId, installId, wants } of pending.values()) {
      changes.push({ row_id: rowId, target_install_id: installId, wants });
    }
    setApplying(true);
    try {
      const summary = await api.applyLibraryChanges(activeKind, changes);
      push(
        `Applied ${summary.copied} change${
          summary.copied === 1 ? "" : "s"
        }, skipped ${summary.skipped}.`,
        "success",
      );
      setPending(new Map());
      await loadKind(activeKind);
      // Also refresh counts so KindNav stays accurate.
      const others = (
        [
          "code_history",
          "cowork_sessions",
          "extensions",
          "mcp_servers",
          "cowork_skills",
          "skills",
          "preferences",
        ] as LibraryKind[]
      ).filter((k) => k !== activeKind);
      for (const k of others) {
        api
          .listLibrary(k)
          .then((rs) =>
            setRowsByKind((current) => ({ ...current, [k]: rs })),
          )
          .catch(() => undefined);
      }
    } catch (e) {
      push(String(e), "error");
    } finally {
      setApplying(false);
    }
  }, [pending, activeKind, loadKind, push]);

  const handleCancel = useCallback(() => setPending(new Map()), []);

  const handleToggleVisible = useCallback((installId: string) => {
    setVisibleIds((current) => {
      const next = new Set(current);
      if (next.has(installId)) next.delete(installId);
      else next.add(installId);
      return next;
    });
  }, []);

  const handleLaunch = useCallback(
    async (install: DesktopInstall) => {
      setBusy(true);
      try {
        await api.launchDesktopInstall(install.id);
        push(`Launching ${install.name}…`, "info");
      } catch (e) {
        push(String(e), "error");
      } finally {
        setBusy(false);
      }
    },
    [push],
  );

  const handleLaunchCodex = useCallback(
    async (install: CodexInstall) => {
      setBusy(true);
      try {
        await api.launchCodexInstall(install.id);
        push(`Launching Codex ${install.name === "default" ? "" : install.name}…`.trim(), "info");
      } catch (e) {
        push(String(e), "error");
      } finally {
        setBusy(false);
      }
    },
    [push],
  );

  const handleSelectProfile = useCallback((install: DesktopInstall) => {
    setSelection((current) =>
      current?.type === "profile" && current.install.id === install.id
        ? null
        : { type: "profile", install },
    );
  }, []);

  const handleSelectRow = useCallback(
    (rowId: string | null) => {
      if (!rowId) {
        setSelection((current) => (current?.type === "row" ? null : current));
        return;
      }
      const row = rowsByKind[activeKind]?.find((r) => r.id === rowId);
      if (!row) return;
      setSelection({ type: "row", row, kind: activeKind });
    },
    [rowsByKind, activeKind],
  );

  // Convert a Codex session (the selected codex_sessions row) into a fresh
  // Claude Code session on disk. We surface the resume command rather than
  // dumping the transcript — the import writes a real ~/.claude/projects file.
  const handleImportCodexSession = useCallback(
    async (sessionId: string) => {
      setImporting(true);
      try {
        const res = await api.importCodexSessionToClaude(sessionId);
        push(
          `Imported ${res.turns} turn${res.turns === 1 ? "" : "s"} → Claude Code. Resume with:  claude --resume ${res.session_id}`,
          "success",
        );
      } catch (e) {
        push(String(e), "error");
      } finally {
        setImporting(false);
      }
    },
    [push],
  );

  // Reverse: export a Claude Code session (resolved by its project cwd) into a
  // fresh Codex rollout. The Codex picker may need a reindex to show it.
  const handleExportClaudeSession = useCallback(
    async (cwd: string, _sessionId: string) => {
      setImporting(true);
      try {
        const res = await api.importClaudeSessionToCodex(cwd);
        push(
          `Exported ${res.turns} turn${res.turns === 1 ? "" : "s"} → Codex. Resume with:  codex resume ${res.session_id}${res.picker === "maybe" ? "  (may need a Codex reindex to appear in the picker)" : ""}`,
          "success",
        );
      } catch (e) {
        push(String(e), "error");
      } finally {
        setImporting(false);
      }
    },
    [push],
  );

  // Each region's "+" creates that type directly — Claude profile (Desktop
  // launcher + Code CLI alias) or Codex profile (Desktop launcher). Both are
  // --user-data-dir launchers; each isolates its own login.
  const handleCreate = useCallback(
    async (kind: "claude" | "codex", rawName: string) => {
      const name = rawName.trim();
      if (!name) return;
      setBusy(true);
      try {
        const parts: string[] = [];
        if (kind === "claude") {
          const desktop = await api.createDesktopProfile(name);
          parts.push(`Desktop launcher (${desktop.name})`);
          const code = await api.createCodeProfile(name, true);
          parts.push(`Code alias ${code.alias_name ?? `claude-${name}`}`);
        } else {
          const codex = await api.createCodexProfile(name);
          parts.push(`Codex launcher (${codex.name})`);
        }
        push(`Created ${kind === "claude" ? "Claude" : "Codex"} profile: ${parts.join(" + ")}.`, "success");
        if (kind === "claude") {
          setClaudeName("");
          setClaudeAdding(false);
        } else {
          setCodexName("");
          setCodexAdding(false);
        }
        await loadInstalls();
      } catch (e) {
        push(String(e), "error");
      } finally {
        setBusy(false);
      }
    },
    [loadInstalls, push],
  );

  const handleDeleteClaude = useCallback(
    async (install: DesktopInstall, deleteData: boolean) => {
      await api.deleteDesktopProfile(install.id, deleteData);
      setVisibleIds((current) => {
        const next = new Set(current);
        next.delete(install.id);
        return next;
      });
      setSelection((current) =>
        current?.type === "profile" && current.install.id === install.id ? null : current,
      );
      push(`Deleted ${install.name}${deleteData ? " — data erased" : ""}.`, "success");
      await loadInstalls();
    },
    [loadInstalls, push],
  );

  const handleDeleteCodex = useCallback(
    async (install: CodexInstall, deleteData: boolean) => {
      await api.deleteCodexProfile(install.id, deleteData);
      push(`Deleted Codex ${install.name}${deleteData ? " — data erased" : ""}.`, "success");
      await loadInstalls();
    },
    [loadInstalls, push],
  );

  const handleTabChange = useCallback((tab: WorkTab) => {
    setActiveTab(tab);
    setPending(new Map());
    setSelection(null);
    setClaudeAdding(false);
    setCodexAdding(false);
    if (tab === "share") {
      setActiveKind("skills");
    } else if (tab === "codex") {
      setActiveKind((k) => (CODEX_KINDS.includes(k) ? k : "codex_sessions"));
    } else if (tab === "claude") {
      setActiveKind((k) => (CLAUDE_KINDS.includes(k) ? k : "code_history"));
    }
  }, []);

  const activeRows = rowsByKind[activeKind] ?? [];
  const selectedRowId =
    selection?.type === "row" ? selection.row.id : null;
  const selectedInstallId =
    selection?.type === "profile" ? selection.install.id : null;

  return (
    <div
      className="flex min-h-0 flex-1"
      data-theme={activeTab === "claude" ? undefined : activeTab}
    >
      {/* Left rail — 3 tool tabs at the very top, then the active tab's body */}
      <aside className="flex w-60 flex-col gap-3 border-r bg-card/30 py-4">
        <div className="px-3">
          <SegmentedTabs value={activeTab} onChange={handleTabChange} />
        </div>

        <div className="scrollbar-thin flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto">
          {activeTab === "claude" ? (
            <>
              <div className="mx-2">
                <SidebarRegion
                  label="Profiles"
                  accent="claude"
                  caption="Desktop + Code. Shares among Claude profiles."
                  count={`${visibleIds.size}/${installs.length}`}
                  adding={claudeAdding}
                  onAddToggle={() => setClaudeAdding((o) => !o)}
                >
                  {claudeAdding ? (
                    <AddProfileInput
                      value={claudeName}
                      onChange={setClaudeName}
                      onConfirm={() => handleCreate("claude", claudeName)}
                      onCancel={() => {
                        setClaudeAdding(false);
                        setClaudeName("");
                      }}
                      busy={busy}
                      hint="New Claude profile — sign in after first launch (quit other Claude windows first)."
                    />
                  ) : null}
                  {sortedInstalls.map((p) => (
                    <SidebarProfileRow
                      key={p.id}
                      profile={p}
                      visible={visibleIds.has(p.id)}
                      selected={selectedInstallId === p.id}
                      onToggleVisible={() => handleToggleVisible(p.id)}
                      onSelect={() => handleSelectProfile(p)}
                      onLaunch={() => handleLaunch(p)}
                      onDelete={(deleteData) => handleDeleteClaude(p, deleteData)}
                      busy={busy}
                    />
                  ))}
                </SidebarRegion>
              </div>
              <div className="mx-2 border-t border-border/60 px-1 pt-3">
                <KindNav
                  value={activeKind}
                  onChange={(k) => {
                    setActiveKind(k);
                    setPending(new Map());
                    setSelection((current) => (current?.type === "row" ? null : current));
                  }}
                  counts={counts}
                  only={CLAUDE_KINDS}
                  heading="Content"
                />
              </div>
            </>
          ) : null}

          {activeTab === "codex" ? (
            <>
            <div className="mx-2">
              <SidebarRegion
                label="Profiles"
                accent="codex"
                caption="Each profile is its own account — separate login + ~/.codex-<name>."
                count={`${codexInstalls.length}`}
                adding={codexAdding}
                onAddToggle={() => setCodexAdding((o) => !o)}
              >
                {codexAdding ? (
                  <AddProfileInput
                    value={codexName}
                    onChange={setCodexName}
                    onConfirm={() => handleCreate("codex", codexName)}
                    onCancel={() => {
                      setCodexAdding(false);
                      setCodexName("");
                    }}
                    busy={busy}
                    hint="New Codex profile — sign in after first launch (quit other Codex windows first)."
                  />
                ) : null}
                {codexInstalls.length === 0 && !codexAdding ? (
                  <p className="px-3 py-2 font-sans text-[12px] text-muted-foreground/70">
                    No Codex profiles yet — + to add one.
                  </p>
                ) : null}
                {codexInstalls.map((c) => (
                  <div
                    key={c.id}
                    className={cn(
                      "group flex items-center gap-1.5 rounded-md pl-1.5 pr-1 transition-colors",
                      c.is_running ? "bg-primary/4" : "hover:bg-muted/60",
                    )}
                  >
                    <span className="relative ml-1 inline-flex h-2 w-2 shrink-0 items-center justify-center">
                      {c.is_running ? (
                        <>
                          <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-primary/60" />
                          <span className="relative inline-flex h-2 w-2 rounded-full bg-primary" />
                        </>
                      ) : (
                        <span
                          className={cn(
                            "inline-block h-1.5 w-1.5 rounded-full",
                            c.kind === "default" ? "bg-muted-foreground/60" : "bg-muted-foreground/30",
                          )}
                        />
                      )}
                    </span>
                    <span
                      className={cn(
                        "flex-1 truncate py-1.5 pl-1 font-sans text-[13px]",
                        c.is_running && "font-medium",
                      )}
                      title={c.data_dir}
                    >
                      {c.kind === "default" ? "Default" : c.name}
                    </span>
                    {c.is_running ? (
                      <span className="mr-1 rounded-full bg-primary/15 px-1.5 py-0.5 font-sans text-[9px] uppercase tracking-wider text-primary">
                        live
                      </span>
                    ) : null}
                    <button
                      type="button"
                      onClick={() => handleLaunchCodex(c)}
                      disabled={busy || c.is_running}
                      className="shrink-0 rounded-md p-1 text-muted-foreground opacity-0 transition-all hover:bg-primary/10 hover:text-primary group-hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-40"
                      title={c.is_running ? `Codex ${c.name} is already running` : `Launch Codex ${c.name}`}
                      aria-label={`Launch Codex ${c.name}`}
                    >
                      <Play className="h-3 w-3" />
                    </button>
                    <DeleteProfileButton
                      name={c.name}
                      kind={c.kind}
                      isRunning={c.is_running}
                      world="codex"
                      busy={busy}
                      onDelete={(deleteData) => handleDeleteCodex(c, deleteData)}
                    />
                  </div>
                ))}
              </SidebarRegion>
            </div>
            <div className="mx-2 border-t border-border/60 px-1 pt-3">
              <KindNav
                value={activeKind}
                onChange={(k) => {
                  setActiveKind(k);
                  setPending(new Map());
                  setSelection((current) => (current?.type === "row" ? null : current));
                }}
                counts={counts}
                only={CODEX_KINDS}
                heading="Content"
              />
            </div>
            </>
          ) : null}

          {activeTab === "share" ? (
            <div className="mx-2">
              <KindNav
                value={activeKind}
                onChange={(k) => {
                  setActiveKind(k);
                  setPending(new Map());
                  setSelection((current) => (current?.type === "row" ? null : current));
                }}
                counts={counts}
                only={["skills", "mcp_cross", "memory_cross"]}
                heading="Cross-tool sharing"
              />
              <p className="mt-1 flex items-start gap-1.5 px-3 font-sans text-[10px] leading-snug text-muted-foreground/70">
                <span className="mt-0.5 h-2 w-2 shrink-0 rounded-[2px] bg-foreground" />
                Skills (SKILL.md) link live between Claude and Codex. MCP servers
                copy across the JSON↔TOML boundary.
              </p>
            </div>
          ) : null}
        </div>
      </aside>

      {/* Center: matrix */}
      <main className="flex min-h-0 flex-1 flex-col gap-2 p-4">
        {toasts.length > 0 ? (
          <div className="space-y-1">
            {toasts.map((toast) => (
              <button
                key={toast.id}
                onClick={() => dismiss(toast.id)}
                className={cn(
                  "block w-full rounded-md border px-3 py-1.5 text-left font-sans text-[12px] transition-colors",
                  toast.kind === "error"
                    ? "border-destructive/40 bg-destructive/10 text-destructive"
                    : toast.kind === "success"
                    ? "border-primary/40 bg-primary/10 text-primary"
                    : "border-border bg-muted/40 text-foreground",
                )}
              >
                {toast.message}
              </button>
            ))}
          </div>
        ) : null}

        {KIND_SCOPE_CAPTION[activeKind] ? (
          <div className="flex items-center gap-2 rounded-md border border-border/60 bg-muted/30 px-3 py-1.5 font-sans text-[11px] text-muted-foreground">
            {activeKind === "skills" || activeKind === "mcp_cross" || activeKind === "memory_cross" ? (
              <span className="h-2 w-2 shrink-0 rounded-[2px] bg-foreground" />
            ) : activeTab === "codex" ? (
              <CodexMark className="h-3 w-3 shrink-0 text-[#4366F2]" />
            ) : null}
            {KIND_SCOPE_CAPTION[activeKind]}
          </div>
        ) : null}

        {matrixColumns.length === 0 ? (
          <div className="flex flex-1 items-center justify-center text-muted-foreground">
            <p className="font-sans text-sm">
              {activeKind === "skills"
                ? "No Claude Code or Codex install found."
                : "No Claude profiles checked — toggle one on the left."}
            </p>
          </div>
        ) : (
          <Matrix
            rows={activeRows}
            profiles={matrixColumns}
            pending={pending}
            onCellToggle={handleCellToggle}
            onRowSelect={handleSelectRow}
            selectedRowId={selectedRowId}
            loading={loadingKind === activeKind}
            emptyHint={EMPTY_HINTS[activeKind]}
          />
        )}
      </main>

      {/* Right rail: profile or row detail */}
      <DetailSheet
        selection={selection}
        onClose={() => setSelection(null)}
        onLaunch={handleLaunch}
        resolveInstallName={resolveInstallName}
        onImportCodexSession={handleImportCodexSession}
        onExportClaudeSession={handleExportClaudeSession}
        transferBusy={importing}
      />

      {/* Floating pending bar */}
      <PendingBar
        count={pending.size}
        applying={applying}
        onApply={handleApply}
        onCancel={handleCancel}
      />
    </div>
  );
}

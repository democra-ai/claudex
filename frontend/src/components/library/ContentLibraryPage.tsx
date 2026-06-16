import { useCallback, useEffect, useMemo, useState } from "react";
import { Info, Play, Plus } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { api, isTauri } from "@/lib/api";
import { useToasts } from "@/hooks/useToast";
import type {
  CodexInstall,
  DesktopInstall,
  LibraryCellChange,
  LibraryKind,
  LibraryRow,
} from "@/types";
import { KindNav, computeKindCount } from "./KindNav";
import { Matrix } from "./Matrix";
import { DetailSheet, type Selection } from "./DetailSheet";
import { PendingBar } from "./PendingBar";

const EMPTY_HINTS: Record<LibraryKind, string> = {
  code_history: "No Cowork code sessions yet in any profile.",
  cowork_sessions: "No Cowork agent-mode sessions in any profile.",
  extensions: "No extensions installed in any profile.",
  mcp_servers: "No MCPs configured in any claude_desktop_config.json.",
  cowork_skills: "No Skills — open Cowork in any profile once.",
  preferences: "Allowlisted preferences not set in any profile.",
};

interface SidebarProfileRowProps {
  profile: DesktopInstall;
  visible: boolean;
  selected: boolean;
  onToggleVisible: () => void;
  onSelect: () => void;
  onLaunch: () => void;
  busy: boolean;
}

function SidebarProfileRow({
  profile,
  visible,
  selected,
  onToggleVisible,
  onSelect,
  onLaunch,
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
    </div>
  );
}

export default function ContentLibraryPage() {
  const [installs, setInstalls] = useState<DesktopInstall[]>([]);
  const [codexInstalls, setCodexInstalls] = useState<CodexInstall[]>([]);
  const [visibleIds, setVisibleIds] = useState<Set<string>>(new Set());
  const [activeKind, setActiveKind] = useState<LibraryKind>("code_history");
  const [rowsByKind, setRowsByKind] = useState<
    Partial<Record<LibraryKind, LibraryRow[]>>
  >({});
  const [pending, setPending] = useState<Map<string, boolean>>(new Map());
  const [selection, setSelection] = useState<Selection>(null);
  const [busy, setBusy] = useState(false);
  const [applying, setApplying] = useState(false);
  const [loadingKind, setLoadingKind] = useState<LibraryKind | null>(null);
  const [newProfileName, setNewProfileName] = useState("");
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
      "preferences",
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
      // Codex installs are an independent dimension — load them alongside but
      // don't fail the whole refresh if Codex isn't present on this machine.
      api.listCodexInstalls().then(setCodexInstalls).catch(() => setCodexInstalls([]));
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
      "preferences",
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
      const key = `${rowId}:${installId}`;
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
          next.set(key, wantsShared);
        }
        return next;
      });
    },
    [rowsByKind, activeKind],
  );

  const handleApply = useCallback(async () => {
    if (pending.size === 0) return;
    const changes: LibraryCellChange[] = [];
    for (const [key, wants] of pending.entries()) {
      const sep = key.lastIndexOf(":");
      const rowId = key.slice(0, sep);
      const installId = key.slice(sep + 1);
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

  const [newWantsDesktop, setNewWantsDesktop] = useState(true);
  const [newWantsCode, setNewWantsCode] = useState(true);
  const [newWantsCodex, setNewWantsCodex] = useState(false);
  const [newSeedCode, setNewSeedCode] = useState(true);

  const handleCreate = useCallback(async () => {
    const name = newProfileName.trim();
    if (!name) return;
    if (!newWantsDesktop && !newWantsCode && !newWantsCodex) {
      push("Pick at least one of Desktop / Code / Codex.", "error");
      return;
    }
    setBusy(true);
    try {
      const parts: string[] = [];
      if (newWantsDesktop) {
        const created = await api.createDesktopProfile(name);
        parts.push(`Desktop launcher (${created.name})`);
      }
      if (newWantsCode) {
        const created = await api.createCodeProfile(name, newSeedCode);
        parts.push(
          `Code CLI alias ${created.alias_name ?? `claude-${name}`}${
            newSeedCode ? " (seeded)" : ""
          }`,
        );
      }
      if (newWantsCodex) {
        const created = await api.createCodexProfile(name, newSeedCode);
        parts.push(`Codex CLI alias ${created.alias_name ?? `codex-${name}`}`);
      }
      push(`Created: ${parts.join(" + ")}.`, "success");
      setNewProfileName("");
      await loadInstalls();
    } catch (e) {
      push(String(e), "error");
    } finally {
      setBusy(false);
    }
  }, [
    newProfileName,
    newWantsDesktop,
    newWantsCode,
    newWantsCodex,
    newSeedCode,
    loadInstalls,
    push,
  ]);

  const activeRows = rowsByKind[activeKind] ?? [];
  const selectedRowId =
    selection?.type === "row" ? selection.row.id : null;
  const selectedInstallId =
    selection?.type === "profile" ? selection.install.id : null;

  return (
    <div className="flex min-h-0 flex-1">
      {/* Left rail */}
      <aside className="flex w-60 flex-col gap-3 border-r bg-card/30 py-4">
        <div className="px-2">
          <KindNav
            value={activeKind}
            onChange={(k) => {
              setActiveKind(k);
              setPending(new Map());
              setSelection((current) =>
                current?.type === "row" ? null : current,
              );
            }}
            counts={counts}
          />
        </div>

        <div className="mx-2 border-t border-border/60 pt-3">
          <div className="mb-1.5 flex items-center justify-between px-3 font-sans text-[10px] uppercase tracking-[0.14em] text-muted-foreground/80">
            <span>Profiles</span>
            <span className="font-mono text-[10px] tabular-nums text-muted-foreground/60">
              {visibleIds.size}/{installs.length}
            </span>
          </div>
          <div className="space-y-0.5 px-1">
            {sortedInstalls.map((p) => (
              <SidebarProfileRow
                key={p.id}
                profile={p}
                visible={visibleIds.has(p.id)}
                selected={selectedInstallId === p.id}
                onToggleVisible={() => handleToggleVisible(p.id)}
                onSelect={() => handleSelectProfile(p)}
                onLaunch={() => handleLaunch(p)}
                busy={busy}
              />
            ))}
          </div>
        </div>

        {codexInstalls.length > 0 ? (
          <div className="mx-2 border-t border-border/60 pt-3">
            <div className="mb-1.5 flex items-center justify-between px-3 font-sans text-[10px] uppercase tracking-[0.14em] text-muted-foreground/80">
              <span>Codex accounts</span>
              <span className="font-mono text-[10px] tabular-nums text-muted-foreground/60">
                {codexInstalls.length}
              </span>
            </div>
            <div className="space-y-0.5 px-1">
              {codexInstalls.map((c) => (
                <div
                  key={c.id}
                  className="flex items-center gap-2 rounded-md px-3 py-1.5"
                  title={c.config_dir}
                >
                  <span
                    className={cn(
                      "h-1.5 w-1.5 shrink-0 rounded-full",
                      c.logged_in ? "bg-foreground/60" : "bg-muted-foreground/30",
                    )}
                  />
                  <div className="flex min-w-0 flex-col leading-tight">
                    <span className="truncate font-sans text-[12px] text-foreground/90">
                      {c.name}
                    </span>
                    <span className="truncate font-mono text-[10px] text-muted-foreground/70">
                      {c.account_email ?? (c.logged_in ? "signed in" : "not signed in")}
                    </span>
                  </div>
                  {c.alias_name ? (
                    <span className="ml-auto shrink-0 font-mono text-[9px] text-muted-foreground/50">
                      {c.alias_name}
                    </span>
                  ) : null}
                </div>
              ))}
            </div>
          </div>
        ) : null}

        <div className="mx-2 mt-auto space-y-2 border-t border-border/60 px-1 pt-3">
          <div className="px-2 font-sans text-[10px] uppercase tracking-[0.14em] text-muted-foreground/80">
            New profile
          </div>
          <div className="flex gap-1">
            <Input
              value={newProfileName}
              onChange={(e) => setNewProfileName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleCreate();
              }}
              placeholder="name"
              className="h-7 font-sans text-xs"
              disabled={busy}
            />
            <Button
              type="button"
              size="icon"
              onClick={handleCreate}
              disabled={busy || !newProfileName.trim()}
              className="h-7 w-7"
              aria-label="Create profile"
            >
              <Plus className="h-3.5 w-3.5" />
            </Button>
          </div>
          {/* Two checkboxes — sets up Desktop launcher AND/OR Code CLI alias
           *  in a single click. If both are on, we run the Desktop and Code
           *  side back-to-back and combine the success toast. */}
          <label className="flex cursor-pointer items-center gap-2 px-2 font-sans text-[11px] text-foreground/85">
            <Checkbox
              checked={newWantsDesktop}
              onCheckedChange={(v) => setNewWantsDesktop(v === true)}
              disabled={busy}
              className="h-3.5 w-3.5"
            />
            Desktop launcher
          </label>
          <label className="flex cursor-pointer items-center gap-2 px-2 font-sans text-[11px] text-foreground/85">
            <Checkbox
              checked={newWantsCode}
              onCheckedChange={(v) => setNewWantsCode(v === true)}
              disabled={busy}
              className="h-3.5 w-3.5"
            />
            Code CLI alias
          </label>
          <label className="flex cursor-pointer items-center gap-2 px-2 font-sans text-[11px] text-foreground/85">
            <Checkbox
              checked={newWantsCodex}
              onCheckedChange={(v) => setNewWantsCodex(v === true)}
              disabled={busy}
              className="h-3.5 w-3.5"
            />
            Codex CLI alias
          </label>
          {newWantsCode || newWantsCodex ? (
            <label
              className={cn(
                "flex cursor-pointer items-start gap-2 pl-6 pr-2 font-sans text-[10px]",
                busy ? "text-muted-foreground/50" : "text-muted-foreground",
              )}
              title="Copy skills, plugins, MCPs from the default config dir. Excludes chat history and credentials."
            >
              <Checkbox
                checked={newSeedCode}
                onCheckedChange={(v) => setNewSeedCode(v === true)}
                disabled={busy}
                className="mt-0.5 h-3 w-3"
              />
              <span>Seed config from default (skills, plugins, MCP)</span>
            </label>
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

        {visibleProfiles.length === 0 ? (
          <div className="flex flex-1 items-center justify-center text-muted-foreground">
            <p className="font-sans text-sm">
              No profiles selected — toggle one on the left.
            </p>
          </div>
        ) : (
          <Matrix
            rows={activeRows}
            profiles={visibleProfiles}
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

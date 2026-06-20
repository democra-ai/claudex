import { useEffect, useState } from "react";
import { ArrowLeftRight, Loader2, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import type {
  DesktopInstall,
  LibraryCell,
  LibraryKind,
  LibraryRow,
  LocalSession,
} from "@/types";
import { Glyph, STATE_LABEL } from "./Glyph";
import { ProfileDetail } from "./ProfileDetail";

export type Selection =
  | { type: "row"; row: LibraryRow; kind: LibraryKind }
  | { type: "profile"; install: DesktopInstall; world?: "claude" | "codex" }
  | null;

interface DetailSheetProps {
  selection: Selection;
  onClose: () => void;
  onLaunch: (install: DesktopInstall) => void;
  resolveInstallName: (installId: string) => string | undefined;
  /** Import a Codex session (by id) into Claude Code. */
  onImportCodexSession?: (sessionId: string) => void;
  /** Export a Claude Code session into Codex (by cwd project + session id). */
  onExportClaudeSession?: (cwd: string, sessionId: string) => void;
  /** True while an import/export round-trip is running. */
  transferBusy?: boolean;
  /** Delete the selected profile (routed by world in the parent). */
  onDeleteProfile?: (deleteData: boolean) => Promise<void>;
  /** Called after a content file is saved/deleted, so the matrix can refresh. */
  onContentChanged?: () => void;
}

/**
 * Right-rail detail panel. Dispatches between a row-level summary (matrix
 * content item) and a profile-level summary (codexbar-style stats).
 * Slides in from the right when something is selected.
 */
export function DetailSheet({
  selection,
  onClose,
  onLaunch,
  resolveInstallName,
  onImportCodexSession,
  onExportClaudeSession,
  transferBusy,
  onDeleteProfile,
  onContentChanged,
}: DetailSheetProps) {
  const visible = selection !== null;
  return (
    <aside
      className={cn(
        "border-l bg-card transition-[width,opacity] duration-220 ease-out",
        visible ? "w-80 opacity-100" : "pointer-events-none w-0 opacity-0",
      )}
    >
      {selection?.type === "profile" ? (
        <ProfileDetail
          install={selection.install}
          onClose={onClose}
          onLaunch={onLaunch}
          resolveName={resolveInstallName}
          world={selection.world ?? "claude"}
          onDelete={onDeleteProfile}
        />
      ) : selection?.type === "row" ? (
        <RowDetail
          row={selection.row}
          kind={selection.kind}
          onClose={onClose}
          resolveInstallName={resolveInstallName}
          onImportCodexSession={onImportCodexSession}
          onExportClaudeSession={onExportClaudeSession}
          transferBusy={transferBusy}
          onContentChanged={onContentChanged}
        />
      ) : null}
    </aside>
  );
}

function formatRelative(ms: number): string {
  const delta = Math.max(0, Date.now() - ms);
  const s = Math.floor(delta / 1000);
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 86400 * 30) return `${Math.floor(s / 86400)}d ago`;
  return `${Math.floor(s / (86400 * 30))}mo ago`;
}

export function SessionList({
  installId,
  installName,
  rowId,
  kind,
  onImportCodexSession,
  onExportClaudeSession,
  transferBusy,
  onContentChanged,
}: {
  installId: string;
  installName: string;
  rowId: string;
  kind: LibraryKind;
  onImportCodexSession?: (sessionId: string) => void;
  onExportClaudeSession?: (cwd: string, sessionId: string) => void;
  transferBusy?: boolean;
  onContentChanged?: () => void;
}) {
  const [sessions, setSessions] = useState<LocalSession[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [reload, setReload] = useState(0);
  const [viewId, setViewId] = useState<string | null>(null);
  const [transcript, setTranscript] = useState<string | null>(null);
  const [confirmDel, setConfirmDel] = useState<string | null>(null);
  const [actBusy, setActBusy] = useState(false);
  const isCodex = kind === "codex_sessions";
  const isClaudeCli = kind === "claude_sessions";
  const isCowork = kind === "cowork_sessions";
  // Which world's transcript/delete commands apply (null = legacy Desktop panel).
  const world = isCodex ? "codex" : isClaudeCli ? "claude" : null;

  useEffect(() => {
    let alive = true;
    setLoading(true);
    const p = isCodex
      ? api.listCodexSessionsForProject(installId, rowId)
      : isClaudeCli
      ? api.listClaudeSessionsForProject(installId, rowId)
      : api.listSessionsForProject(installId, rowId, isCowork);
    p.then((s) => {
      if (alive) setSessions(s);
    })
      .catch(() => {
        if (alive) setSessions([]);
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [installId, rowId, isCodex, isClaudeCli, isCowork, reload]);

  const viewTranscript = (sid: string) => {
    if (!world) return;
    setViewId(sid);
    setTranscript(null);
    api
      .getSessionTranscript(installId, sid, world)
      .then((t) => setTranscript(t))
      .catch((e) => setTranscript(`(${String(e)})`));
  };
  const deleteSession = async (sid: string) => {
    if (!world) return;
    setActBusy(true);
    try {
      await api.deleteSessionFile(installId, sid, world);
      setConfirmDel(null);
      setReload((r) => r + 1);
      onContentChanged?.();
    } catch {
      /* ignore */
    } finally {
      setActBusy(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center gap-1.5 px-1 py-2 text-muted-foreground">
        <Loader2 className="h-3 w-3 animate-spin" />
        <span className="font-sans text-[10px]">Loading sessions…</span>
      </div>
    );
  }
  if (!sessions || sessions.length === 0) {
    return (
      <div className="px-1 py-2 font-sans text-[10px] text-muted-foreground/70">
        No sessions in {installName}.
      </div>
    );
  }
  return (
    <>
      {/* Export is project-level: the converter reads the Claude *CLI* transcript
          (~/.claude/projects/<slug>), not the per-session Desktop panel store,
          so we export the newest CLI session for this project. */}
      {kind === "code_history" && onExportClaudeSession ? (
        <button
          type="button"
          disabled={transferBusy}
          onClick={() => onExportClaudeSession(rowId, "")}
          /* Destination-coloured: Codex indigo. */
          className="mt-1 inline-flex items-center gap-1 rounded bg-[#4366F2]/12 px-1.5 py-0.5 font-sans text-[10px] text-[#4366F2] transition-colors hover:bg-[#4366F2]/22 disabled:opacity-50"
        >
          <ArrowLeftRight className="h-3 w-3" />
          Export newest → Codex
        </button>
      ) : null}
      <ul className="mt-1 space-y-1">
      {sessions.slice(0, 12).map((s) => (
        <li key={s.session_id} className="rounded-md bg-background/60 px-2 py-1.5">
          <div className="line-clamp-2 font-sans text-[11px] text-foreground/90">
            {s.title || (
              <span className="italic text-muted-foreground">
                {s.session_id.slice(0, 8)}…
              </span>
            )}
          </div>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-2 font-mono text-[9px] text-muted-foreground/70">
            {s.last_activity_ms ? (
              <span>{formatRelative(s.last_activity_ms)}</span>
            ) : null}
            {s.model ? <span>{s.model.replace(/\[.*\]$/, "")}</span> : null}
          </div>
          {world ? (
            <div className="mt-1.5 flex flex-wrap items-center gap-1">
              {isCodex && onImportCodexSession ? (
                <button
                  type="button"
                  disabled={transferBusy}
                  onClick={() => onImportCodexSession(s.session_id)}
                  /* Destination-coloured: Claude copper. */
                  className="inline-flex items-center gap-1 rounded bg-[#c96442]/12 px-1.5 py-0.5 font-sans text-[10px] text-[#c96442] transition-colors hover:bg-[#c96442]/22 disabled:opacity-50"
                >
                  <ArrowLeftRight className="h-3 w-3" />
                  Import to Claude
                </button>
              ) : null}
              {isClaudeCli && onExportClaudeSession ? (
                <button
                  type="button"
                  disabled={transferBusy}
                  onClick={() => onExportClaudeSession(rowId, s.session_id)}
                  /* Destination-coloured: Codex indigo. */
                  className="inline-flex items-center gap-1 rounded bg-[#4366F2]/12 px-1.5 py-0.5 font-sans text-[10px] text-[#4366F2] transition-colors hover:bg-[#4366F2]/22 disabled:opacity-50"
                >
                  <ArrowLeftRight className="h-3 w-3" />
                  Export to Codex
                </button>
              ) : null}
              <button
                type="button"
                onClick={() => (viewId === s.session_id ? setViewId(null) : viewTranscript(s.session_id))}
                className="rounded bg-muted px-1.5 py-0.5 font-sans text-[10px] text-foreground/70 hover:bg-muted/70"
              >
                {viewId === s.session_id ? "Hide" : "View"}
              </button>
              {confirmDel === s.session_id ? (
                <>
                  <button
                    type="button"
                    disabled={actBusy}
                    onClick={() => deleteSession(s.session_id)}
                    className="rounded bg-destructive/90 px-1.5 py-0.5 font-sans text-[10px] text-destructive-foreground disabled:opacity-50"
                  >
                    Confirm
                  </button>
                  <button
                    type="button"
                    onClick={() => setConfirmDel(null)}
                    className="font-sans text-[10px] text-muted-foreground"
                  >
                    cancel
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  onClick={() => setConfirmDel(s.session_id)}
                  className="rounded px-1.5 py-0.5 font-sans text-[10px] text-destructive hover:bg-destructive/10"
                >
                  Delete
                </button>
              )}
            </div>
          ) : null}
          {viewId === s.session_id ? (
            <pre className="scrollbar-thin mt-1.5 max-h-56 overflow-auto whitespace-pre-wrap rounded bg-muted/40 p-2 font-mono text-[9.5px] leading-relaxed text-foreground/80">
              {transcript ?? "Loading transcript…"}
            </pre>
          ) : null}
        </li>
      ))}
      {sessions.length > 12 ? (
        <li className="px-1 py-1 font-sans text-[10px] text-muted-foreground/70">
          +{sessions.length - 12} more…
        </li>
      ) : null}
      </ul>
    </>
  );
}

type ContentTarget =
  | { mode: "file"; path: string; label: string }
  | { mode: "mcp"; configPath: string; server: string; label: string };

/** Resolve the editable content behind a matrix cell, or null if not editable. */
function contentTargetFor(
  kind: LibraryKind,
  cell: LibraryCell,
  rowId: string,
): ContentTarget | null {
  if (kind === "memory" || kind === "codex_memory" || kind === "memory_cross") {
    return { mode: "file", path: cell.data_dir, label: cell.kind === "codex" ? "AGENTS.md" : "CLAUDE.md" };
  }
  if (kind === "skills" || kind === "codex_skills" || kind === "claude_skills") {
    return { mode: "file", path: `${cell.data_dir}/${rowId}/SKILL.md`, label: "SKILL.md" };
  }
  if (kind === "mcp_servers" || kind === "codex_mcp" || kind === "mcp_cross") {
    // Desktop MCP lives in claude_desktop_config.json under the data dir; the
    // other MCP kinds carry the config file directly in data_dir.
    const configPath =
      kind === "mcp_servers"
        ? `${cell.data_dir}/claude_desktop_config.json`
        : cell.data_dir;
    return { mode: "mcp", configPath, server: rowId, label: `MCP · ${rowId}` };
  }
  return null;
}

/** View + edit + delete the file behind a cell. Replaces the cell list while open. */
function ContentPanel({
  installName,
  target,
  isLink,
  onBack,
  onChanged,
}: {
  installName: string;
  target: ContentTarget;
  isLink: boolean;
  onBack: () => void;
  onChanged: () => void;
}) {
  const [content, setContent] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const isMcp = target.mode === "mcp";

  const read = () =>
    target.mode === "mcp"
      ? api.readMcpServer(target.configPath, target.server)
      : api.readTextFile(target.path);

  useEffect(() => {
    let alive = true;
    read()
      .then((c) => {
        if (alive) {
          setContent(c);
          setDraft(c);
        }
      })
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target.mode, target.mode === "mcp" ? target.configPath : target.path]);

  const dirty = content !== null && draft !== content;
  const empty = content === "";

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      if (target.mode === "mcp") {
        await api.writeMcpServer(target.configPath, target.server, draft);
      } else {
        await api.writeTextFile(target.path, draft);
      }
      setContent(draft);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const del = async () => {
    setBusy(true);
    setError(null);
    try {
      if (target.mode === "mcp") {
        await api.deleteMcpServer(target.configPath, target.server);
      } else {
        await api.deleteContentPath(target.path);
      }
      onChanged();
      onBack();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full flex-col">
      <div className="mb-2 flex items-center justify-between gap-2">
        <button
          type="button"
          onClick={onBack}
          className="font-sans text-[11px] text-muted-foreground hover:text-foreground"
        >
          ← back
        </button>
        <span className="truncate font-mono text-[10px] text-muted-foreground/70">
          {installName} · {target.label}
        </span>
      </div>
      {!isMcp && isLink ? (
        <div className="mb-2 rounded bg-state-shared/8 px-2 py-1 font-sans text-[10px] text-state-shared">
          Shared (symlink) — un-share to edit independently. Saving is blocked.
        </div>
      ) : null}
      {content === null && !error ? (
        <div className="flex items-center gap-1.5 py-3 text-muted-foreground">
          <Loader2 className="h-3 w-3 animate-spin" />
          <span className="font-sans text-[10px]">Loading…</span>
        </div>
      ) : (
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          spellCheck={false}
          placeholder={empty ? "Empty — type to create this file…" : ""}
          className="scrollbar-thin min-h-0 flex-1 resize-none rounded-md border bg-background px-2.5 py-2 font-mono text-[11px] leading-relaxed text-foreground/90 outline-none focus:ring-1 focus:ring-ring"
        />
      )}
      {error ? (
        <p className="mt-2 font-mono text-[10px] leading-snug text-destructive">{error}</p>
      ) : null}
      <div className="mt-2 flex items-center justify-between gap-2">
        {confirmDelete ? (
          <span className="flex items-center gap-1.5">
            <button
              type="button"
              disabled={busy}
              onClick={del}
              className="rounded bg-destructive/90 px-2 py-1 font-sans text-[11px] text-destructive-foreground disabled:opacity-50"
            >
              Confirm delete
            </button>
            <button
              type="button"
              onClick={() => setConfirmDelete(false)}
              className="font-sans text-[11px] text-muted-foreground"
            >
              cancel
            </button>
          </span>
        ) : (
          <button
            type="button"
            disabled={busy || empty}
            onClick={() => setConfirmDelete(true)}
            className="rounded px-2 py-1 font-sans text-[11px] text-destructive hover:bg-destructive/10 disabled:opacity-40"
          >
            Delete
          </button>
        )}
        <button
          type="button"
          disabled={busy || !dirty || (!isMcp && isLink)}
          onClick={save}
          className="inline-flex items-center gap-1 rounded bg-primary px-2.5 py-1 font-sans text-[11px] text-primary-foreground disabled:opacity-40"
        >
          {busy ? <Loader2 className="h-3 w-3 animate-spin" /> : null}
          {empty ? "Create" : "Save"}
        </button>
      </div>
    </div>
  );
}

function RowDetail({
  row,
  kind,
  onClose,
  resolveInstallName,
  onImportCodexSession,
  onExportClaudeSession,
  transferBusy,
  onContentChanged,
}: {
  row: LibraryRow;
  kind: LibraryKind;
  onClose: () => void;
  resolveInstallName: (installId: string) => string | undefined;
  onImportCodexSession?: (sessionId: string) => void;
  onExportClaudeSession?: (cwd: string, sessionId: string) => void;
  transferBusy?: boolean;
  onContentChanged?: () => void;
}) {
  const [openCell, setOpenCell] = useState<LibraryCell | null>(null);
  const [importing, setImporting] = useState<string | null>(null);
  const [importErr, setImportErr] = useState<string | null>(null);
  const showSessions =
    ((kind === "code_history" || kind === "cowork_sessions") &&
      row.id !== "__workspace__") ||
    ((kind === "codex_sessions" || kind === "claude_sessions") &&
      row.id !== "__all_sessions__");
  const isMemoryKind =
    kind === "memory" || kind === "codex_memory" || kind === "memory_cross";
  const handleImportMemory = async (src: LibraryCell, dst: LibraryCell) => {
    const key = `${src.install_id}->${dst.install_id}`;
    setImporting(key);
    setImportErr(null);
    try {
      await api.importMemoryFile(src.data_dir, dst.data_dir);
      onContentChanged?.();
    } catch (e) {
      setImportErr(String(e));
    } finally {
      setImporting(null);
    }
  };
  const openTarget = openCell ? contentTargetFor(kind, openCell, row.id) : null;
  return (
    <div className="sheet-slide flex h-full flex-col">
      <header className="flex items-start justify-between gap-2 border-b px-4 py-3">
        <div className="min-w-0">
          <div className="font-sans text-[10px] uppercase tracking-[0.14em] text-muted-foreground">
            Item
          </div>
          <div className="mt-0.5 truncate font-display text-lg leading-tight">
            {row.label}
          </div>
          {row.label !== row.id ? (
            <div className="truncate font-mono text-[10px] text-muted-foreground/80">
              {row.id}
            </div>
          ) : null}
          {row.description ? (
            <p className="mt-1.5 font-sans text-xs text-muted-foreground/90">
              {row.description}
            </p>
          ) : null}
        </div>
        <button
          type="button"
          onClick={onClose}
          className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          aria-label="Close details"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </header>
      <div className="scrollbar-thin flex-1 overflow-y-auto px-4 py-3">
        {openCell && openTarget ? (
          <ContentPanel
            installName={resolveInstallName(openCell.install_id) ?? openCell.install_name}
            target={openTarget}
            isLink={openCell.state === "shared"}
            onBack={() => setOpenCell(null)}
            onChanged={() => onContentChanged?.()}
          />
        ) : (
        <>
        {importErr ? (
          <div className="mb-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-1.5 font-sans text-[11px] text-destructive">
            {importErr}
          </div>
        ) : null}
        <ul className="space-y-1.5">
          {row.cells.map((cell) => {
            const editable = contentTargetFor(kind, cell, row.id) !== null;
            return (
            <li
              key={cell.install_id}
              onClick={editable ? () => setOpenCell(cell) : undefined}
              className={cn(
                "rounded-md bg-muted/30 px-3 py-2",
                editable && "cursor-pointer hover:bg-muted/60",
              )}
            >
              <div className="mb-1 flex items-center justify-between gap-2">
                <span className="truncate font-sans text-xs text-foreground/85">
                  {resolveInstallName(cell.install_id) ?? cell.install_name}
                </span>
                <span className="flex items-center gap-1.5">
                  <Glyph state={cell.state} size="sm" />
                  <span className="font-sans text-[10px] text-muted-foreground">
                    {STATE_LABEL[cell.state].toLowerCase()}
                  </span>
                </span>
              </div>
              {cell.detail ? (
                <div className="break-words font-sans text-[11px] text-foreground/80">
                  {cell.detail}
                </div>
              ) : (
                <div className="font-mono text-[11px] text-muted-foreground/60">
                  —
                </div>
              )}
              {cell.digest || cell.link_target_digest ? (
                <div className="mt-1 flex gap-2 font-mono text-[9px] text-muted-foreground/60">
                  {cell.digest ? <span>val:{cell.digest.slice(0, 8)}</span> : null}
                  {cell.link_target_digest ? (
                    <span>link:{cell.link_target_digest.slice(0, 8)}</span>
                  ) : null}
                </div>
              ) : null}
              {showSessions && cell.present ? (
                <SessionList
                  installId={cell.install_id}
                  installName={
                    resolveInstallName(cell.install_id) ?? cell.install_name
                  }
                  rowId={row.id}
                  kind={kind}
                  onImportCodexSession={onImportCodexSession}
                  onExportClaudeSession={onExportClaudeSession}
                  transferBusy={transferBusy}
                  onContentChanged={onContentChanged}
                />
              ) : null}
              {isMemoryKind && cell.present ? (
                <div
                  className="mt-1.5 flex flex-wrap items-center gap-1.5"
                  onClick={(e) => e.stopPropagation()}
                >
                  <span className="font-sans text-[10px] text-muted-foreground">
                    Import to
                  </span>
                  {row.cells
                    .filter(
                      (c) =>
                        c.install_id !== cell.install_id &&
                        c.data_dir !== cell.data_dir,
                    )
                    .map((other) => {
                      const key = `${cell.install_id}->${other.install_id}`;
                      return (
                        <button
                          key={other.install_id}
                          type="button"
                          disabled={importing !== null}
                          onClick={() => handleImportMemory(cell, other)}
                          className="rounded border border-border px-1.5 py-0.5 font-sans text-[10px] text-foreground/80 transition-colors hover:bg-muted disabled:opacity-50"
                        >
                          {importing === key ? "…" : "→"}{" "}
                          {resolveInstallName(other.install_id) ?? other.install_name}
                        </button>
                      );
                    })}
                </div>
              ) : null}
              {editable ? (
                <div className="mt-1 font-sans text-[10px] text-primary/80">
                  {cell.present ? "view · edit ›" : "create ›"}
                </div>
              ) : null}
            </li>
            );
          })}
        </ul>
        </>
        )}
      </div>
    </div>
  );
}

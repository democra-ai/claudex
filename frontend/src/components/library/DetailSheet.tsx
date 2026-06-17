import { useEffect, useState } from "react";
import { ArrowLeftRight, Loader2, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import type {
  DesktopInstall,
  LibraryKind,
  LibraryRow,
  LocalSession,
} from "@/types";
import { Glyph, STATE_LABEL } from "./Glyph";
import { ProfileDetail } from "./ProfileDetail";

export type Selection =
  | { type: "row"; row: LibraryRow; kind: LibraryKind }
  | { type: "profile"; install: DesktopInstall }
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

function SessionList({
  installId,
  installName,
  rowId,
  kind,
  onImportCodexSession,
  onExportClaudeSession,
  transferBusy,
}: {
  installId: string;
  installName: string;
  rowId: string;
  kind: LibraryKind;
  onImportCodexSession?: (sessionId: string) => void;
  onExportClaudeSession?: (cwd: string, sessionId: string) => void;
  transferBusy?: boolean;
}) {
  const [sessions, setSessions] = useState<LocalSession[] | null>(null);
  const [loading, setLoading] = useState(true);
  const isCodex = kind === "codex_sessions";
  const isCowork = kind === "cowork_sessions";

  useEffect(() => {
    let alive = true;
    setLoading(true);
    const p = isCodex
      ? api.listCodexSessionsForProject(installId, rowId)
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
  }, [installId, rowId, isCodex, isCowork]);

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
          className="mt-1 inline-flex items-center gap-1 rounded bg-foreground/10 px-1.5 py-0.5 font-sans text-[10px] text-foreground/80 transition-colors hover:bg-foreground/20 disabled:opacity-50"
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
          {isCodex && onImportCodexSession ? (
            <button
              type="button"
              disabled={transferBusy}
              onClick={() => onImportCodexSession(s.session_id)}
              className="mt-1.5 inline-flex items-center gap-1 rounded bg-primary/10 px-1.5 py-0.5 font-sans text-[10px] text-primary transition-colors hover:bg-primary/20 disabled:opacity-50"
            >
              <ArrowLeftRight className="h-3 w-3" />
              Import to Claude
            </button>
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

function RowDetail({
  row,
  kind,
  onClose,
  resolveInstallName,
  onImportCodexSession,
  onExportClaudeSession,
  transferBusy,
}: {
  row: LibraryRow;
  kind: LibraryKind;
  onClose: () => void;
  resolveInstallName: (installId: string) => string | undefined;
  onImportCodexSession?: (sessionId: string) => void;
  onExportClaudeSession?: (cwd: string, sessionId: string) => void;
  transferBusy?: boolean;
}) {
  const showSessions =
    ((kind === "code_history" || kind === "cowork_sessions") &&
      row.id !== "__workspace__") ||
    (kind === "codex_sessions" && row.id !== "__all_sessions__");
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
        <ul className="space-y-1.5">
          {row.cells.map((cell) => (
            <li
              key={cell.install_id}
              className="rounded-md bg-muted/30 px-3 py-2"
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
                />
              ) : null}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

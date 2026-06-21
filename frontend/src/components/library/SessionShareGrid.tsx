import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { profileColorVars } from "@/lib/profileColor";
import type { CellState, SessionShareRow, SessionShareCell } from "@/types";
import { Glyph } from "./Glyph";
import { pendingKeyFor, type PendingChange } from "./pending";

interface SessionShareGridProps {
  /** "codex_sessions" | "claude_sessions" */
  kind: string;
  /** The project row id (cwd / worktree root). */
  projectId: string;
  pending: Map<string, PendingChange>;
  /** Stage a per-session toggle. `currentShared` is the real (on-disk) state. */
  onToggle: (
    rowId: string,
    installId: string,
    currentShared: boolean,
    wants: boolean,
  ) => void;
  /** Bump to force a reload after Apply. */
  refreshKey: number;
}

/** Predicted post-toggle glyph for a session cell (pending-aware), mirroring
 *  MatrixCell.predictedState on the share axis. */
function predicted(cell: SessionShareCell, wants: boolean | undefined): CellState {
  if (wants === undefined) return cell.state;
  const currentlyShared = cell.state === "shared";
  if (wants === currentlyShared) return cell.state;
  if (wants) return "shared";
  return cell.state !== "absent" ? "independent" : "absent";
}

/**
 * Per-session share grid: one row per session in a project, one cell per account.
 * Click a cell to toggle whether THAT session is symlink-shared into THAT account.
 * Disabled (dim) when the account's whole space is already shared, or the session
 * is still active (would risk two accounts co-writing one file).
 */
export function SessionShareGrid({
  kind,
  projectId,
  pending,
  onToggle,
  refreshKey,
}: SessionShareGridProps) {
  const [rows, setRows] = useState<SessionShareRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setRows(null);
    setError(null);
    api
      .listSessionShareGrid(kind, projectId)
      .then((r) => {
        if (!cancelled) setRows(r);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [kind, projectId, refreshKey]);

  if (error) {
    return (
      <div className="font-mono text-[10px] text-destructive/80">{error}</div>
    );
  }
  if (!rows) {
    return (
      <div className="flex items-center gap-1.5 font-sans text-[11px] text-muted-foreground/70">
        <Loader2 className="h-3 w-3 animate-spin" /> loading sessions…
      </div>
    );
  }
  if (rows.length === 0) return null;

  // Column order from the first session (every row carries all accounts).
  const cols = rows[0].cells;

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="font-sans text-[10px] font-medium uppercase tracking-wider text-muted-foreground/70">
          Share individual sessions
        </span>
        <span className="font-mono text-[9px] text-muted-foreground/50">
          click a dot to share · active sessions locked
        </span>
      </div>
      <div className="overflow-hidden rounded border">
        {/* header */}
        <div
          className="grid items-center border-b bg-muted/30 px-2 py-1"
          style={{ gridTemplateColumns: `1fr repeat(${cols.length}, 2.2rem)` }}
        >
          <span className="font-mono text-[9px] text-muted-foreground/60">session</span>
          {cols.map((c) => (
            <span
              key={c.install_id}
              className="truncate text-center font-sans text-[9px] font-medium text-foreground/80"
              title={c.install_name}
            >
              {c.install_name.replace(/^Default.*/, "Def")}
            </span>
          ))}
        </div>
        {/* one row per session */}
        {rows.map((s) => (
          <div
            key={s.session_id}
            className="grid items-center border-b px-2 py-1 last:border-b-0 hover:bg-muted/20"
            style={{ gridTemplateColumns: `1fr repeat(${cols.length}, 2.2rem)` }}
          >
            <span className="flex min-w-0 items-center gap-1.5">
              <span className="truncate font-sans text-[11px] text-foreground/85" title={s.title ?? s.session_id}>
                {s.title ?? s.session_id}
              </span>
              {s.active ? (
                <span className="shrink-0 rounded-full bg-amber-500/15 px-1 py-px font-sans text-[8px] uppercase tracking-wider text-amber-600">
                  active
                </span>
              ) : null}
            </span>
            {s.cells.map((cell) => {
              const rowId = `sess:${projectId}:${s.session_id}`;
              const key = pendingKeyFor(rowId, cell.install_id);
              const desired = pending.get(key)?.wants;
              const isPending = desired !== undefined;
              const eff = predicted(cell, desired);
              const disabled = !cell.actionable || s.active;
              const currentShared = cell.state === "shared";
              return (
                <button
                  key={cell.install_id}
                  type="button"
                  disabled={disabled}
                  onClick={() =>
                    onToggle(rowId, cell.install_id, currentShared, !(eff === "shared"))
                  }
                  title={
                    !cell.actionable
                      ? `${cell.install_name}: whole space already shared`
                      : s.active
                        ? "Session is active — finish it before sharing"
                        : eff === "shared"
                          ? `Shared with ${cell.install_name} — click to unshare`
                          : `Click to share with ${cell.install_name}`
                  }
                  className={cn(
                    "flex h-6 items-center justify-center rounded transition-colors",
                    disabled
                      ? "cursor-not-allowed opacity-30"
                      : "hover:bg-primary/8",
                    isPending && "bg-amber-500/10 ring-1 ring-inset ring-amber-500/30",
                  )}
                >
                  <Glyph state={eff} size="sm" profileVars={profileColorVars(cell.install_id)} />
                </button>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}

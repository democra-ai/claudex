import { useCallback, useEffect, useState } from "react";
import { History, RotateCcw, Shield, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api, isTauri } from "@/lib/api";
import type { BackupManifest } from "@/lib/api";

interface BackupsPanelProps {
  open: boolean;
  onClose: () => void;
  /** Called after a successful restore so the page can reload its matrices. */
  onRestored: () => void;
}

function humanBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

const REASON_LABEL: Record<string, string> = {
  startup: "On launch",
  "before-apply": "Before Apply",
  "before-delete": "Before delete",
  "before-change": "Before change",
  "before-restore": "Before restore",
  manual: "Manual",
};

/**
 * Backups / Restore. Lists the local snapshots the app takes automatically
 * (at launch + before every Apply) and lets the user roll all managed content
 * back to any of them. Restore is itself snapshotted first, so it's reversible.
 */
export function BackupsPanel({ open, onClose, onRestored }: BackupsPanelProps) {
  const [backups, setBackups] = useState<BackupManifest[]>([]);
  const [loading, setLoading] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    if (!isTauri()) return;
    setLoading(true);
    api
      .listBackups()
      .then((list) => setBackups(list))
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    if (open) {
      setError(null);
      setConfirmId(null);
      reload();
    }
  }, [open, reload]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const backupNow = useCallback(() => {
    setBusyId("__new__");
    setError(null);
    api
      .createBackup("manual", "Manual backup")
      .then(() => reload())
      .catch((e) => setError(String(e)))
      .finally(() => setBusyId(null));
  }, [reload]);

  const restore = useCallback(
    (id: string) => {
      setBusyId(id);
      setError(null);
      api
        .restoreBackup(id)
        .then(() => {
          setConfirmId(null);
          onRestored();
          onClose();
        })
        .catch((e) => setError(String(e)))
        .finally(() => setBusyId(null));
    },
    [onRestored, onClose],
  );

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="flex max-h-[80vh] w-[min(620px,92vw)] flex-col overflow-hidden rounded-2xl border border-border/60 bg-card shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center justify-between border-b border-border/50 px-6 py-4">
          <div className="flex items-center gap-2.5">
            <Shield className="h-5 w-5 text-primary" />
            <div>
              <h2 className="text-base font-semibold">Backups &amp; Restore</h2>
              <p className="text-xs text-muted-foreground">
                Auto-saved at launch and before every Apply · last 15 kept
              </p>
            </div>
          </div>
          <Button variant="ghost" size="icon" onClick={onClose} aria-label="Close">
            <X className="h-4 w-4" />
          </Button>
        </header>

        <div className="flex items-center justify-between gap-3 border-b border-border/40 px-6 py-3">
          <p className="text-xs text-muted-foreground">
            Restoring rolls <strong>all</strong> managed content (sessions, skills, memory, MCP,
            preferences) back to that point. The current state is snapshotted first.
          </p>
          <Button size="sm" variant="secondary" onClick={backupNow} disabled={busyId === "__new__"}>
            <Shield className="mr-1.5 h-3.5 w-3.5" />
            {busyId === "__new__" ? "Backing up…" : "Back up now"}
          </Button>
        </div>

        {error ? (
          <div className="mx-6 mt-3 rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {error}
          </div>
        ) : null}

        <div className="flex-1 overflow-y-auto px-3 py-2">
          {loading && backups.length === 0 ? (
            <p className="px-3 py-8 text-center text-sm text-muted-foreground">Loading…</p>
          ) : backups.length === 0 ? (
            <div className="flex flex-col items-center gap-2 px-3 py-10 text-center text-sm text-muted-foreground">
              <History className="h-6 w-6 opacity-50" />
              No snapshots yet — one is taken automatically on launch.
            </div>
          ) : (
            <ul className="flex flex-col gap-1.5">
              {backups.map((b) => {
                const confirming = confirmId === b.id;
                const busy = busyId === b.id;
                return (
                  <li
                    key={b.id}
                    className="flex items-center justify-between gap-3 rounded-lg border border-border/40 bg-background/40 px-4 py-3"
                  >
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-sm font-medium">{b.label}</span>
                        <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
                          {REASON_LABEL[b.reason] ?? b.reason}
                        </span>
                      </div>
                      <div className="mt-0.5 text-xs text-muted-foreground">
                        {new Date(b.createdAtMs).toLocaleString()} · {b.totalFiles} files ·{" "}
                        {humanBytes(b.totalBytes)}
                      </div>
                    </div>
                    {confirming ? (
                      <div className="flex shrink-0 items-center gap-1.5">
                        <Button size="sm" variant="destructive" onClick={() => restore(b.id)} disabled={busy}>
                          {busy ? "Restoring…" : "Confirm restore"}
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => setConfirmId(null)} disabled={busy}>
                          Cancel
                        </Button>
                      </div>
                    ) : (
                      <Button
                        size="sm"
                        variant="outline"
                        className="shrink-0"
                        onClick={() => setConfirmId(b.id)}
                      >
                        <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
                        Restore
                      </Button>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

import { useCallback, useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ArrowUpCircle, Download, Loader2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { isTauri } from "@/lib/api";

type Phase = "hidden" | "available" | "downloading" | "ready" | "error";

/**
 * Auto-update prompt. On launch we ask the GitHub `latest.json` endpoint whether
 * a newer signed build exists; if so a slim non-blocking bar appears. "Install &
 * restart" downloads the update (with progress), verifies its signature, swaps
 * the app, and relaunches. Offline / no-update / no-endpoint just stays hidden.
 */
export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [phase, setPhase] = useState<Phase>("hidden");
  const [pct, setPct] = useState(0);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    check()
      .then((u) => {
        if (!cancelled && u?.available) {
          setUpdate(u);
          setPhase("available");
        }
      })
      .catch(() => undefined); // offline / endpoint unreachable → silent
    return () => {
      cancelled = true;
    };
  }, []);

  const install = useCallback(async () => {
    if (!update) return;
    setPhase("downloading");
    setErr(null);
    setPct(0);
    let total = 0;
    let got = 0;
    try {
      await update.downloadAndInstall((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? 0;
        else if (ev.event === "Progress") {
          got += ev.data.chunkLength;
          if (total) setPct(Math.min(99, Math.round((got / total) * 100)));
        } else if (ev.event === "Finished") setPct(100);
      });
      setPhase("ready");
      await relaunch();
    } catch (e) {
      setErr(String(e));
      setPhase("error");
    }
  }, [update]);

  if (phase === "hidden" || !update) return null;

  return (
    <div className="flex items-center gap-3 border-b border-primary/30 bg-primary/10 px-4 py-2 text-sm">
      <ArrowUpCircle className="h-4 w-4 shrink-0 text-primary" />
      {phase === "available" ? (
        <>
          <span className="min-w-0 flex-1 truncate">
            <strong>Claudex {update.version}</strong> is available
            <span className="text-muted-foreground"> · you're on {update.currentVersion}</span>
          </span>
          <Button size="sm" onClick={install} className="h-7 gap-1.5 text-xs">
            <Download className="h-3 w-3" />
            Install &amp; restart
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setPhase("hidden")} className="h-7 w-7 p-0" aria-label="Later">
            <X className="h-3.5 w-3.5" />
          </Button>
        </>
      ) : phase === "downloading" || phase === "ready" ? (
        <>
          <span className="min-w-0 flex-1">
            {phase === "ready" ? "Installing — restarting…" : `Downloading Claudex ${update.version}…`}
          </span>
          <div className="hidden h-1.5 w-40 overflow-hidden rounded-full bg-primary/20 sm:block">
            <div className="h-full rounded-full bg-primary transition-all" style={{ width: `${pct}%` }} />
          </div>
          <span className="w-9 text-right tabular-nums text-muted-foreground">{pct}%</span>
          <Loader2 className="h-4 w-4 animate-spin text-primary" />
        </>
      ) : (
        <>
          <span className="min-w-0 flex-1 truncate text-destructive">Update failed — {err}</span>
          <Button size="sm" variant="outline" onClick={install} className="h-7 text-xs">
            Retry
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setPhase("hidden")} className="h-7 w-7 p-0" aria-label="Dismiss">
            <X className="h-3.5 w-3.5" />
          </Button>
        </>
      )}
    </div>
  );
}

import { useEffect, useRef, useState } from "react";
import { Loader2, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { cn } from "@/lib/utils";

interface DeleteProfileButtonProps {
  name: string;
  /** "default" = the original install; deletable now, but only by erasing the
   *  real data dir (it has no launcher). All profiles are equal. */
  kind: "default" | "profile";
  isRunning: boolean;
  world: "claude" | "codex";
  busy: boolean;
  /** Resolve to delete; reject with a message to keep the popover open. */
  onDelete: (deleteData: boolean) => Promise<void>;
  /** "icon" = hover-trash (legacy); "full" = a labelled button for the detail
   *  panel. Defaults to "full" now that delete lives in the detail sheet. */
  variant?: "icon" | "full";
}

/**
 * Confirmation popover for deleting a profile. Safe destructive UX: lists exactly
 * what's removed, makes data-dir erasure an explicit opt-in, and surfaces backend
 * errors inline. The default install has no launcher, so deleting it REQUIRES the
 * data-erase opt-in (and the backend guards the path).
 */
export function DeleteProfileButton({
  name,
  kind,
  isRunning,
  world,
  busy,
  onDelete,
  variant = "full",
}: DeleteProfileButtonProps) {
  const [open, setOpen] = useState(false);
  const [deleteData, setDeleteData] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
        setError(null);
        setDeleteData(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const isDefault = kind === "default";

  const artifacts = isDefault
    ? world === "claude"
      ? ["The real ~/Library Claude data dir — your primary login + all chats"]
      : ["The real Codex data dir + ~/.codex agent home (login, sessions)"]
    : world === "claude"
    ? ["Launcher app", `Code CLI alias claude-${name}`, "Registry entry"]
    : ["Launcher app", "Registry entry"];

  // The default has nothing to remove but data, so erasing is required.
  const canConfirm = !isDefault || deleteData;

  const confirm = async () => {
    setDeleting(true);
    setError(null);
    try {
      await onDelete(deleteData);
      setOpen(false);
      setDeleteData(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <div className={cn("relative", variant === "full" ? "w-full" : "shrink-0")} ref={ref}>
      {variant === "icon" ? (
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          disabled={busy || isRunning}
          className={cn(
            "rounded-md p-1 text-muted-foreground transition-all hover:bg-destructive/10 hover:text-destructive disabled:cursor-not-allowed disabled:opacity-40",
            open ? "opacity-100" : "opacity-0 group-hover:opacity-100",
          )}
          title={isRunning ? `Quit ${name} before deleting` : `Delete ${name}`}
          aria-label={`Delete ${name}`}
        >
          <Trash2 className="h-3 w-3" />
        </button>
      ) : (
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          disabled={busy || isRunning}
          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 font-sans text-[12px] text-destructive transition-colors hover:bg-destructive/10 disabled:cursor-not-allowed disabled:opacity-40"
          title={isRunning ? `Quit ${name} before deleting` : `Delete ${name}`}
        >
          <Trash2 className="h-3.5 w-3.5" />
          {isRunning ? "Quit to delete" : "Delete profile…"}
        </button>
      )}

      {open ? (
        <div
          className={cn(
            "absolute z-50 w-64 rounded-md border bg-background p-3 shadow-lg",
            variant === "full" ? "right-0 bottom-9" : "right-0 top-7",
          )}
        >
          <p className="font-sans text-[12px] text-foreground">
            Delete <span className="font-medium">“{name}”</span>? This can’t be undone.
          </p>
          <ul className="mt-2 space-y-0.5">
            {artifacts.map((a) => (
              <li
                key={a}
                className="flex items-start gap-1.5 font-sans text-[10px] text-muted-foreground"
              >
                <span className="mt-1 h-1 w-1 shrink-0 rounded-full bg-muted-foreground/50" />
                {a}
              </li>
            ))}
          </ul>
          {!isDefault ? (
            <label className="mt-2.5 flex cursor-pointer items-start gap-2 font-sans text-[11px] text-foreground/85">
              <Checkbox
                checked={deleteData}
                onCheckedChange={(v) => setDeleteData(v === true)}
                disabled={deleting}
                className="mt-0.5 h-3.5 w-3.5"
              />
              <span>
                {world === "claude"
                  ? "Also delete login + chats (data dir)"
                  : "Also delete login (data dir)"}
              </span>
            </label>
          ) : (
            <label className="mt-2.5 flex cursor-pointer items-start gap-2 rounded-md bg-destructive/5 p-2 font-sans text-[11px] text-destructive">
              <Checkbox
                checked={deleteData}
                onCheckedChange={(v) => setDeleteData(v === true)}
                disabled={deleting}
                className="mt-0.5 h-3.5 w-3.5"
              />
              <span>
                I understand this erases the <span className="font-medium">real</span>{" "}
                {world === "claude" ? "Claude" : "Codex"} install — its login and all
                data.
              </span>
            </label>
          )}
          {error ? (
            <p className="mt-2 font-mono text-[10px] leading-snug text-destructive">{error}</p>
          ) : null}
          <div className="mt-3 flex justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-[11px]"
              onClick={() => {
                setOpen(false);
                setError(null);
                setDeleteData(false);
              }}
              disabled={deleting}
            >
              Cancel
            </Button>
            <Button
              type="button"
              variant="destructive"
              size="sm"
              className="h-6 gap-1.5 px-2 text-[11px]"
              onClick={confirm}
              disabled={deleting || !canConfirm}
            >
              {deleting ? <Loader2 className="h-3 w-3 animate-spin" /> : null}
              {deleteData ? "Delete + erase data" : "Delete launcher"}
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

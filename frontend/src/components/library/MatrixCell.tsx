import { useMemo } from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { CellState, LibraryCell } from "@/types";
import { Glyph, STATE_LABEL } from "./Glyph";
import { pendingKeyFor, type PendingChange } from "./pending";
import { profileColorVars } from "@/lib/profileColor";

interface MatrixCellProps {
  cell: LibraryCell;
  rowId: string;
  /** Staged toggles keyed by pendingKeyFor(rowId, install_id). */
  pending: Map<string, PendingChange>;
  /** `wantsShared` true = make this cell shared (link to a source sibling);
   *  false = make it independent (keep its own copy, leave the link). */
  onToggle: (rowId: string, installId: string, wantsShared: boolean) => void;
  /** Non-interactive cells render dimmer and don't stage a pending toggle. */
  interactive?: boolean;
}

/** Predicted post-toggle state of the cell. Used to render the pending glyph
 *  optimistically while keeping the original state in `cell.state`.
 *
 *  The toggle is on the SHARE axis: `wantsShared` true = join the link group
 *  (→ shared), false = leave it but keep your own copy (→ independent, or
 *  absent if the cell held nothing). Real state is recomputed on the next
 *  refresh — copy-kind nuances (copied vs shared, MCP refuse-on-diverge,
 *  memory un-share → absent) self-correct then; this is a best-effort
 *  single-glyph hint mid-edit, not a per-family simulation. */
function predictedState(
  cell: LibraryCell,
  wantsShared: boolean | undefined,
): CellState {
  if (wantsShared === undefined) return cell.state;
  const currentlyShared = cell.state === "shared" || cell.state === "copied";
  if (wantsShared === currentlyShared) return cell.state;
  if (wantsShared) return "shared";
  return cell.present ? "independent" : "absent";
}

export function MatrixCell({
  cell,
  rowId,
  pending,
  onToggle,
  interactive = true,
}: MatrixCellProps) {
  const pendingKey = pendingKeyFor(rowId, cell.install_id);
  const desired = pending.get(pendingKey)?.wants;
  const isPending = interactive && desired !== undefined;
  const effectiveState = interactive
    ? predictedState(cell, desired)
    : cell.state;

  const tooltip = useMemo(() => {
    const lines: string[] = [cell.install_name];
    const stateLine =
      STATE_LABEL[effectiveState] +
      (isPending ? " · pending" : "") +
      (!interactive ? " · browse only" : "");
    lines.push(stateLine);
    if (cell.detail) lines.push(cell.detail);
    return lines.join(" — ");
  }, [cell, effectiveState, isPending, interactive]);

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          onClick={() =>
            onToggle(
              rowId,
              cell.install_id,
              // Share-axis toggle, pending-aware: flip the EFFECTIVE shared-ness
              // (which already reflects a staged change) so a second click
              // sends the opposite intent and cancels the pending entry.
              // shared/copied → make independent (false); independent / absent
              // / diverged → share (true).
              !(effectiveState === "shared" || effectiveState === "copied"),
            )
          }
          className={cn(
            // `min-h-10` keeps the 40px floor for short rows; `h-full` lets
            // tall rows (e.g. session-title labels that wrap to 2 lines)
            // stretch the cell to match, so the glyph stays centered.
            "flex min-h-10 h-full w-full items-center justify-center transition-colors",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            interactive ? "hover:bg-primary/8" : "hover:bg-muted/40 cursor-default",
            isPending && "bg-amber-500/10 ring-1 ring-inset ring-amber-500/30",
          )}
          aria-label={tooltip}
          type="button"
        >
          <Glyph
            state={effectiveState}
            profileVars={profileColorVars(cell.install_id)}
          />
        </button>
      </TooltipTrigger>
      <TooltipContent side="top" className="font-mono text-[11px]">
        {tooltip}
      </TooltipContent>
    </Tooltip>
  );
}

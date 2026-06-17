import { useMemo, useState } from "react";
import { Search, Inbox } from "lucide-react";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import type { DesktopInstall, LibraryRow } from "@/types";
import { MatrixCell } from "./MatrixCell";
import type { PendingChange } from "./pending";
import { Glyph, STATE_GLYPH } from "./Glyph";

interface MatrixProps {
  rows: LibraryRow[];
  /** Profiles to render as columns, ordered. */
  profiles: DesktopInstall[];
  pending: Map<string, PendingChange>;
  onCellToggle: (rowId: string, installId: string, nextPresent: boolean) => void;
  onRowSelect: (rowId: string | null) => void;
  selectedRowId: string | null;
  loading: boolean;
  /** Optional empty-state message specific to the kind. */
  emptyHint?: string;
}

/**
 * The Grid — high-density matrix of (item × profile) cells.
 *
 * Layout:
 *   row 1: column headers (profile chips). Sticky top.
 *   row 2…N: content rows. Sticky left column = content label.
 *
 * Density rules:
 *   - Profile chips: uppercase JBM, no padding wider than the glyph beneath.
 *   - Row labels: lowercase mono, dim until row hover.
 *   - Cells: 36px tall, glyph centered, no border between cells horizontally
 *     (the row separator alone carries the grid feel).
 */
export function Matrix({
  rows,
  profiles,
  pending,
  onCellToggle,
  onRowSelect,
  selectedRowId,
  loading,
  emptyHint,
}: MatrixProps) {
  const [search, setSearch] = useState("");

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return rows;
    return rows.filter(
      (row) =>
        row.id.toLowerCase().includes(needle) ||
        row.label.toLowerCase().includes(needle) ||
        (row.description?.toLowerCase().includes(needle) ?? false),
    );
  }, [rows, search]);

  // Per-profile alignment summary shown in column header.
  const columnSummary = useMemo(() => {
    return profiles.map((p) => {
      let present = 0;
      let shared = 0;
      let copied = 0;
      for (const row of rows) {
        const cell = row.cells.find((c) => c.install_id === p.id);
        if (!cell) continue;
        if (cell.present) present++;
        if (cell.state === "shared") shared++;
        if (cell.state === "copied") copied++;
      }
      return { installId: p.id, present, shared, copied };
    });
  }, [profiles, rows]);

  // Grid template: 280px for the label column, then equal-width for each profile.
  const gridTemplate = `280px repeat(${profiles.length}, minmax(72px, 1fr))`;

  // Count rows in each group for the section-header meta line.
  const groupCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const r of filtered) {
      const g = r.group ?? "";
      m.set(g, (m.get(g) ?? 0) + 1);
    }
    return m;
  }, [filtered]);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded border bg-card">
      {/* Sticky column header. Filter input lives inside the Item cell —
       *  one combined strip instead of an extra filter bar above. */}
      <div
        className="grid border-b bg-muted/30"
        style={{ gridTemplateColumns: gridTemplate }}
      >
        <div className="relative border-r px-3 py-2">
          <Search className="pointer-events-none absolute left-5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={`Filter ${rows.length} item${rows.length === 1 ? "" : "s"}`}
            className="h-7 border-transparent bg-transparent pl-7 font-sans text-xs focus:bg-background focus:border-input"
          />
        </div>
        {profiles.map((p, i) => {
          const summary = columnSummary[i];
          const isCodex = p.id === "codex:global";
          return (
            <div
              key={p.id}
              className={cn(
                "flex flex-col items-center justify-center px-1 py-2 leading-none",
                p.kind === "default" && "bg-muted/40",
                // The global Codex skills column: tint + a left wall so it
                // never reads as just another Claude profile.
                isCodex && "border-l-2 border-border bg-foreground/5",
              )}
              title={
                isCodex
                  ? "Codex — ~/.codex/skills (global, shared by all Codex profiles)"
                  : `${p.name}\n${summary.present} present · ${summary.shared} shared · ${summary.copied} copied`
              }
            >
              <span className="truncate font-sans text-[12px] font-medium text-foreground/90">
                {p.kind === "default" ? "Default" : p.name}
              </span>
              {isCodex ? (
                <span className="font-mono text-[8px] text-muted-foreground/60">global</span>
              ) : null}
              <span className="mt-0.5 font-mono text-[9px] tabular-nums text-muted-foreground/70">
                {summary.shared > 0 ? (
                  <span className="text-state-shared">
                    {STATE_GLYPH.shared} {summary.shared}
                  </span>
                ) : null}
                {summary.copied > 0 ? (
                  <span className="ml-1 text-state-copied">
                    {STATE_GLYPH.copied} {summary.copied}
                  </span>
                ) : null}
                {summary.shared === 0 && summary.copied === 0 ? (
                  <span>
                    {summary.present}/{rows.length}
                  </span>
                ) : null}
              </span>
            </div>
          );
        })}
      </div>

      {/* Body */}
      <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto">
        {loading ? (
          <div className="space-y-1 p-3 text-muted-foreground">
            {Array.from({ length: 8 }).map((_, i) => (
              <div key={i} className="h-10 animate-pulse rounded-md bg-muted/40" />
            ))}
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 py-16 text-center text-muted-foreground">
            <Inbox className="h-8 w-8 opacity-50" />
            <p className="font-sans text-sm">{emptyHint ?? "Nothing here yet."}</p>
          </div>
        ) : (
          <div className="stagger-children">
            {(() => {
              let lastGroup: string | null | undefined = undefined;
              const out: React.ReactNode[] = [];
              filtered.forEach((row, rowIdx) => {
                const isSelected = selectedRowId === row.id;
                // Insert a section header whenever the group flips.
                if (row.group !== lastGroup) {
                  lastGroup = row.group;
                  if (row.group) {
                    const count = groupCounts.get(row.group) ?? 0;
                    out.push(
                      <div
                        key={`__group__${row.group}`}
                        className="flex items-baseline justify-between gap-3 border-b bg-muted/30 px-4 py-1.5"
                      >
                        <h3 className="font-sans text-[10px] font-medium uppercase tracking-[0.16em] text-foreground/85">
                          {row.group}
                        </h3>
                        <span className="font-mono text-[10px] tabular-nums text-muted-foreground/70">
                          {count} {count === 1 ? "row" : "rows"}
                        </span>
                      </div>,
                    );
                  }
                }
                out.push(
                  <div
                    key={row.id}
                    className={cn(
                      "grid items-stretch grid-divider transition-colors",
                      isSelected
                        ? "bg-accent/15"
                        : "hover:bg-accent/8",
                    )}
                    style={{
                      gridTemplateColumns: gridTemplate,
                      animationDelay: `${Math.min(rowIdx * 12, 240)}ms`,
                    }}
                  >
                  {/* Row label — `min-w-0` lets it respect the grid column
                   *  instead of expanding to its intrinsic content width.
                   *  `overflow-hidden` confines the truncate/line-clamp. */}
                  <button
                    type="button"
                    onClick={() => onRowSelect(isSelected ? null : row.id)}
                    className={cn(
                      "flex min-w-0 flex-col items-start justify-center gap-0.5 overflow-hidden border-r px-4 py-2 text-left transition-colors",
                      "hover:bg-muted/40",
                      isSelected && "border-l-2 border-l-primary",
                    )}
                    title={
                      row.label !== row.id
                        ? `${row.label}\n${row.id}`
                        : row.id
                    }
                  >
                    {/* Label: wrap up to 2 lines for session titles, then ellipsis.
                     *  `block w-full` makes line-clamp actually box-constrained
                     *  rather than overflowing into the cells column. */}
                    <span
                      className={cn(
                        "block w-full font-sans text-[13px] leading-snug line-clamp-2 break-words",
                        isSelected
                          ? "font-medium text-foreground"
                          : "text-foreground/85",
                      )}
                    >
                      {row.label}
                    </span>
                    {row.description ? (
                      <span className="block w-full truncate font-sans text-[10px] text-muted-foreground/80">
                        {row.description}
                      </span>
                    ) : row.label !== row.id ? (
                      <span className="block w-full truncate font-mono text-[10px] text-muted-foreground/70">
                        {row.id}
                      </span>
                    ) : null}
                  </button>
                  {/* Cells */}
                  {profiles.map((p) => {
                    const cell = row.cells.find((c) => c.install_id === p.id);
                    if (!cell) {
                      return (
                        <div
                          key={p.id}
                          className="flex items-center justify-center"
                        >
                          <Glyph state="absent" />
                        </div>
                      );
                    }
                    return (
                      <MatrixCell
                        key={p.id}
                        cell={cell}
                        rowId={row.id}
                        pending={pending}
                        interactive={row.interactive}
                        onToggle={(rowId, installId, nextPresent) => {
                          if (row.interactive) {
                            onCellToggle(rowId, installId, nextPresent);
                          } else {
                            // Browse-only rows: cell click opens detail
                            // for that row instead of staging a toggle.
                            onRowSelect(rowId);
                          }
                        }}
                      />
                    );
                  })}
                  </div>,
                );
              });
              return out;
            })()}
          </div>
        )}
      </div>
    </div>
  );
}

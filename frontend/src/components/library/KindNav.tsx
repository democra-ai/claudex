import { Blocks, Boxes, Hammer, MessagesSquare, Sparkles, Settings2, Wrench } from "lucide-react";
import { cn } from "@/lib/utils";
import type { LibraryKind, LibraryRow } from "@/types";

type KindDef = {
  value: LibraryKind;
  label: string;
  icon: typeof Blocks;
  /** Short hint shown beneath the label. */
  blurb: string;
  /** True for the one kind that shares across the Claude<->Codex boundary. */
  crossTool?: boolean;
};

const KINDS: KindDef[] = [
  {
    value: "code_history",
    label: "Code sessions",
    icon: MessagesSquare,
    blurb: "Cowork chat panel",
  },
  {
    value: "cowork_sessions",
    label: "Cowork sessions",
    icon: Sparkles,
    blurb: "Agent mode runs",
  },
  {
    value: "extensions",
    label: "Extensions",
    icon: Blocks,
    blurb: "Desktop add-ons",
  },
  {
    value: "mcp_servers",
    label: "MCPs",
    icon: Boxes,
    blurb: "claude_desktop_config.json",
  },
  {
    value: "cowork_skills",
    label: "Cowork skills",
    icon: Hammer,
    blurb: "local-agent skills",
  },
  {
    value: "skills",
    label: "Skills",
    icon: Wrench,
    blurb: "~/.claude/skills + Codex",
    crossTool: true,
  },
  {
    value: "preferences",
    label: "Preferences",
    icon: Settings2,
    blurb: "allowlisted prefs",
  },
];

interface KindNavProps {
  value: LibraryKind;
  onChange: (kind: LibraryKind) => void;
  /** Per-kind counts of {synced, total} for the badge — null = unknown yet. */
  counts: Partial<Record<LibraryKind, { synced: number; total: number } | null>>;
}

/** Compute "synced" count for a kind given its rows. A row is "synced" if at
 *  least one cell is shared OR copied; this is a quick alignment-health hint. */
export function computeKindCount(rows: LibraryRow[]): {
  synced: number;
  total: number;
} {
  let synced = 0;
  for (const row of rows) {
    if (row.cells.some((c) => c.state === "shared" || c.state === "copied")) {
      synced++;
    }
  }
  return { synced, total: rows.length };
}

export function KindNav({ value, onChange, counts }: KindNavProps) {
  return (
    <nav className="flex flex-col gap-0.5">
      <div className="px-3 pb-2 font-sans text-[10px] uppercase tracking-[0.14em] text-muted-foreground/70">
        Content
      </div>
      {KINDS.map((kind) => {
        const Icon = kind.icon;
        const active = value === kind.value;
        const count = counts[kind.value];
        return (
          <button
            key={kind.value}
            type="button"
            onClick={() => onChange(kind.value)}
            className={cn(
              "group flex items-center justify-between gap-2 rounded-md px-3 py-2 text-left transition-colors",
              active
                ? "bg-primary/10 text-foreground"
                : "text-foreground/75 hover:bg-muted hover:text-foreground",
            )}
          >
            <span className="flex min-w-0 items-center gap-2.5">
              <Icon
                className={cn(
                  "h-3.5 w-3.5 shrink-0",
                  active ? "text-primary" : "text-muted-foreground",
                )}
              />
              <span className="flex flex-col leading-tight">
                <span className="flex items-center gap-1.5 font-sans text-[13px]">
                  {kind.label}
                  {kind.crossTool ? (
                    <span
                      className="h-2 w-2 shrink-0 rounded-[2px] bg-foreground"
                      title="Shares across Claude and Codex"
                    />
                  ) : null}
                </span>
                <span className="font-sans text-[10px] text-muted-foreground/70">
                  {kind.blurb}
                </span>
              </span>
            </span>
            {count ? (
              <span
                className={cn(
                  "font-mono text-[10px] tabular-nums",
                  count.synced > 0 && active
                    ? "text-primary"
                    : "text-muted-foreground/70",
                )}
              >
                {count.synced}/{count.total}
              </span>
            ) : null}
          </button>
        );
      })}
    </nav>
  );
}

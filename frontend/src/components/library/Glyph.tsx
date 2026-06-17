import type { CSSProperties } from "react";
import { cn } from "@/lib/utils";
import type { CellState } from "@/types";

/**
 * State glyphs for the Content Library matrix. Double-encoded — symbol + color —
 * so the matrix reads correctly at distance and with colorblindness.
 *
 *   ■ shared       lime   live symlink, edits propagate
 *   ●  copied      stone  one-shot copy, value matches
 *   ◐ diverged     amber  both sides present, values differ
 *   ○ independent  grey   present here, not aligned anywhere
 *   ·  absent      dim    not in this profile
 */

export const STATE_GLYPH: Record<CellState, string> = {
  shared: "■",
  copied: "●",
  diverged: "◐",
  independent: "○",
  absent: "·",
};

export const STATE_LABEL: Record<CellState, string> = {
  shared: "Shared",
  copied: "Copied",
  diverged: "Diverged",
  independent: "Independent",
  absent: "Absent",
};

export const STATE_COLOR: Record<CellState, string> = {
  shared: "text-state-shared",
  copied: "text-state-copied",
  diverged: "text-state-diverged",
  independent: "text-state-independent",
  absent: "text-state-absent",
};

interface GlyphProps {
  state: CellState;
  size?: "sm" | "md" | "lg";
  className?: string;
  /** Per-profile identity colour vars (--profile-h/s/l). Tints the
   *  "independent" glyph so each account is recognisable; ignored for shared
   *  (green glow), copied/diverged (copy-mode colours), and absent (muted). */
  profileVars?: CSSProperties;
}

export function Glyph({ state, size = "md", className, profileVars }: GlyphProps) {
  // "shared" is the universal green glowing dot, regardless of profile colour.
  if (state === "shared") {
    return (
      <span
        aria-label={STATE_LABEL.shared}
        className={cn("dot-shared align-middle", className)}
      />
    );
  }
  // "independent" carries the column's identity colour; copy-mode + absent keep
  // their state colours.
  const useProfile = state === "independent";
  return (
    <span
      aria-label={STATE_LABEL[state]}
      style={useProfile ? profileVars : undefined}
      className={cn(
        "font-mono tabular-nums leading-none select-none glyph-swap",
        useProfile ? "profile-ident" : STATE_COLOR[state],
        size === "sm" && "text-[14px]",
        size === "md" && "text-[18px]",
        size === "lg" && "text-[22px]",
        className,
      )}
    >
      {STATE_GLYPH[state]}
    </span>
  );
}

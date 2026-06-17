import type { CSSProperties } from "react";

/**
 * Deterministic per-profile identity colours. Each account/column gets one of 8
 * distinct hues (hashed from its install id, so it's stable across renders and
 * sessions). The green band (h ~90–160) is deliberately excluded — green is
 * reserved for the universal "shared" marker, so an independent column can never
 * be confused with a shared one.
 */
const PROFILE_HUES = [38, 350, 268, 205, 320, 188, 18, 232]; // amber, rose, violet, sky, magenta, teal-cyan, orange, indigo
const PROFILE_SAT = [70, 64, 60, 64, 66, 58, 72, 64];
const PROFILE_LIGHT = [48, 54, 58, 50, 54, 44, 50, 52];

function djb2(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) >>> 0;
  return h;
}

export function profileColorIndex(id: string): number {
  return djb2(id) % PROFILE_HUES.length;
}

/** Inline CSS vars (--profile-h/s/l) consumed by `.profile-ident` / `.profile-bg`. */
export function profileColorVars(id: string): CSSProperties {
  const i = profileColorIndex(id);
  return {
    ["--profile-h" as string]: String(PROFILE_HUES[i]),
    ["--profile-s" as string]: `${PROFILE_SAT[i]}%`,
    ["--profile-l" as string]: `${PROFILE_LIGHT[i]}%`,
  } as CSSProperties;
}

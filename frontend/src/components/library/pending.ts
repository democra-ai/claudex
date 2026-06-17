/** A staged matrix toggle. Stored structured so the (rowId, installId) never
 *  has to be recovered by splitting an ambiguous colon-joined string — install
 *  ids are namespaced (claude-code:default, profile:<name>, codex:profile:<name>)
 *  and some row ids contain colons (preferences "scope:key"). */
export type PendingChange = {
  rowId: string;
  installId: string;
  wants: boolean;
};

/** Map key for a pending toggle — a NUL delimiter that can't appear in any row
 *  id or namespaced install id, so distinct (rowId, installId) pairs can never
 *  collide into the same key. */
export function pendingKeyFor(rowId: string, installId: string): string {
  return rowId + String.fromCharCode(0) + installId;
}

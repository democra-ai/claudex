import { useCallback, useState } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Toolbar } from "@/components/Toolbar";
import { BackupsPanel } from "@/components/BackupsPanel";
import ContentLibraryPage from "@/components/library/ContentLibraryPage";

/**
 * Root shell — minimal chrome over the Content Library page.
 *
 * The old A/B pair-wise UX was retired in favor of a global matrix view that
 * shows every content item across every profile at once. See
 * docs/plans/2026-05-27-content-library-grid.md for the design rationale.
 *
 * Pair-wise share APIs are still exposed by the backend (and the new library
 * apply path delegates to them under the hood) — only the UI changed.
 */
export default function App() {
  // The refresh button lives in the Toolbar but the actual work happens in
  // ContentLibraryPage. We bridge with a key-bump to remount cleanly.
  const [refreshKey, setRefreshKey] = useState(0);
  const handleRefresh = useCallback(() => setRefreshKey((k) => k + 1), []);
  const [backupsOpen, setBackupsOpen] = useState(false);

  return (
    <TooltipProvider delayDuration={200}>
      {/* No bg here — the body's diagonal cream→blue gradient (index.css) must
          read through. A solid `bg-background` used to sit here and cover it,
          flattening the whole canvas to one color. Panels (the bg-card/30
          sidebar, the floating white matrix card) sit over the gradient. */}
      <div className="flex h-full flex-col">
        <Toolbar
          onRefresh={handleRefresh}
          busy={false}
          onOpenBackups={() => setBackupsOpen(true)}
        />
        <ContentLibraryPage key={refreshKey} />
      </div>
      <BackupsPanel
        open={backupsOpen}
        onClose={() => setBackupsOpen(false)}
        onRestored={handleRefresh}
      />
    </TooltipProvider>
  );
}

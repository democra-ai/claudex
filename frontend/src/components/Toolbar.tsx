import { Moon, RefreshCw, Sun, SunMoon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { useTheme } from "@/hooks/useTheme";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

interface ToolbarProps {
  onRefresh: () => void;
  busy: boolean;
}

/**
 * Top chrome. Title, refresh, theme toggle. The Apply action lives on the
 * floating PendingBar inside the page body — keeping it out of the chrome
 * means the user's hand stays close to the matrix when toggling.
 */
export function Toolbar({ onRefresh, busy }: ToolbarProps) {
  const { theme, toggle } = useTheme();
  const ThemeIcon = theme === "dark" ? Moon : theme === "light" ? Sun : SunMoon;

  return (
    // `data-tauri-drag-region` makes the bar draggable via Tauri's native
    // macOS drag handler (NSWindow.performWindowDragWithEvent). We
    // deliberately don't use CSS `-webkit-app-region: drag` because that
    // hits a known macOS Sonoma+ WebKit bug where drag works once and then
    // gets stuck. The attribute path is bug-free. Interactive children
    // (buttons) don't carry the attribute, so Tauri skips drag on them.
    <header
      data-tauri-drag-region
      className="flex h-12 items-center justify-between border-b bg-background/95 pl-[88px] pr-5 backdrop-blur supports-[backdrop-filter]:bg-background/60"
    >
      <div
        data-tauri-drag-region
        className="flex items-center gap-2.5"
      >
        <div
          data-tauri-drag-region
          className="flex h-6 w-6 items-center justify-center rounded-md bg-primary font-display text-[13px] font-semibold text-primary-foreground"
        >
          C
        </div>
        <h1
          data-tauri-drag-region
          className="font-display text-[15px] leading-none tracking-tight"
        >
          Claude Multiprofile
        </h1>
      </div>

      <div className="flex items-center gap-1">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              onClick={toggle}
              aria-label="Toggle theme"
              className="h-8 w-8 rounded-md"
            >
              <ThemeIcon className="h-3.5 w-3.5" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Theme: {theme}</TooltipContent>
        </Tooltip>
        <Separator orientation="vertical" className="h-4" />
        <Button
          variant="ghost"
          size="sm"
          onClick={onRefresh}
          disabled={busy}
          className="h-8 gap-1.5 rounded-md font-sans text-xs"
        >
          <RefreshCw className={busy ? "h-3 w-3 animate-spin" : "h-3 w-3"} />
          Refresh
        </Button>
      </div>
    </header>
  );
}

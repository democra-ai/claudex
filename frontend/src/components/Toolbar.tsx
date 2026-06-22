import { Moon, RefreshCw, Shield, Sun, SunMoon } from "lucide-react";
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
  onOpenBackups: () => void;
}

/**
 * Top chrome. Title, refresh, theme toggle. The Apply action lives on the
 * floating PendingBar inside the page body — keeping it out of the chrome
 * means the user's hand stays close to the matrix when toggling.
 */
export function Toolbar({ onRefresh, busy, onOpenBackups }: ToolbarProps) {
  const { theme, toggle } = useTheme();
  const ThemeIcon = theme === "dark" ? Moon : theme === "light" ? Sun : SunMoon;

  // Title-bar exact values lifted from Claude.app's compiled main bundle:
  //   trafficLightPosition: { x: 17, y: 17 }   <- Math.round((45 - 12) / 2)
  //   titleBarStyle: "hidden"                  <- Electron specific
  //   bar height: 45 px                        <- Qgt
  //   light height: 12 px                      <- dgt
  // Source: /Applications/Claude.app/Contents/Resources/app.asar
  //         (.vite/build/index.js, function Ver()).
  //
  // Tauri 2 has no "hidden" equivalent; "Overlay" + transparent: true
  // is the closest. With y=17 the lights' center sits at y=23. With
  // h-[45px] + items-center my content center sits at y=22.5 — same
  // baseline as Claude.app's chrome.
  return (
    <header
      data-tauri-drag-region
      className="flex h-[45px] items-center justify-between border-b border-border/50 bg-transparent pl-[80px] pr-4"
    >
      <div data-tauri-drag-region className="flex items-center gap-2">
        {/* The Claude×Codex split mark. Kept at 16px with a rounded mask so it
         *  sits in scale with the title and the action icons, not looming. */}
        <img
          data-tauri-drag-region
          src="/logo-mark.png"
          width="16"
          height="16"
          alt="Claudex"
          className="rounded-[4px]"
          style={{ imageRendering: "auto" }}
        />
        <h1
          data-tauri-drag-region
          className="font-display text-[14px] leading-none tracking-tight"
        >
          Claudex
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
              className="h-7 w-7 rounded-md"
            >
              <ThemeIcon className="h-3.5 w-3.5" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Theme: {theme}</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="sm"
              onClick={onOpenBackups}
              aria-label="Backups and restore"
              className="h-7 gap-1.5 rounded-md font-sans text-xs"
            >
              <Shield className="h-3 w-3" />
              Backups
            </Button>
          </TooltipTrigger>
          <TooltipContent>Backups &amp; restore — roll back to a snapshot</TooltipContent>
        </Tooltip>
        <Separator orientation="vertical" className="h-4" />
        <Button
          variant="ghost"
          size="sm"
          onClick={onRefresh}
          disabled={busy}
          className="h-7 gap-1.5 rounded-md font-sans text-xs"
        >
          <RefreshCw className={busy ? "h-3 w-3 animate-spin" : "h-3 w-3"} />
          Refresh
        </Button>
      </div>
    </header>
  );
}

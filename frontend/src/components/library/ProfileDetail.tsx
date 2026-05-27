import { useEffect, useState } from "react";
import {
  AtSign,
  Calendar,
  Cpu,
  Database,
  Folder,
  Hammer,
  Hash,
  HardDrive,
  Blocks,
  Boxes,
  Loader2,
  MessagesSquare,
  Monitor,
  Network,
  Play,
  Server,
  Sparkles,
  TrendingDown,
  TrendingUp,
  User,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { DesktopInstall, ProfileStats } from "@/types";
import { UsageBar } from "./UsageBar";

interface ProfileDetailProps {
  install: DesktopInstall;
  onClose: () => void;
  onLaunch: (install: DesktopInstall) => void;
  resolveName: (installId: string) => string | undefined;
}

function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

function formatNumber(n: number): string {
  return n.toLocaleString();
}

function formatRelativeTime(ms: number | null | undefined): string {
  if (!ms) return "—";
  const delta = Math.max(0, Date.now() - ms);
  const s = Math.floor(delta / 1000);
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 86400 * 30) return `${Math.floor(s / 86400)}d ago`;
  return `${Math.floor(s / (86400 * 30))}mo ago`;
}

function formatDate(ms: number | null | undefined): string {
  if (!ms) return "—";
  const d = new Date(ms);
  return d.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function tildify(p: string): string {
  return p.replace(/^\/Users\/[^/]+/, "~");
}

function todayISO(): string {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** Rough plan-tier inference from daily token usage. Honest about being
 *  an estimate — labels as "≈ Heavy" rather than "Max" so we don't fake
 *  knowledge we don't have. */
function planInference(tokensToday: number): {
  label: string;
  tone: "muted" | "high";
} {
  if (tokensToday >= 1_000_000) return { label: "Heavy day", tone: "high" };
  if (tokensToday >= 250_000) return { label: "Active", tone: "high" };
  if (tokensToday >= 50_000) return { label: "Moderate", tone: "muted" };
  if (tokensToday > 0) return { label: "Light", tone: "muted" };
  return { label: "Idle", tone: "muted" };
}

/** Friendly delta label vs baseline. Returns null when baseline is zero
 *  or insufficient history. */
function paceVsBaseline(
  today: number,
  baseline: number,
): { text: string; up: boolean } | null {
  if (baseline <= 0) return null;
  const ratio = today / baseline;
  if (today === 0) {
    return { text: `Below pace — baseline ${baseline.toFixed(1)}/d`, up: false };
  }
  const pct = Math.round((ratio - 1) * 100);
  if (Math.abs(pct) < 8) {
    return { text: `On pace — baseline ${baseline.toFixed(1)}/d`, up: true };
  }
  if (pct >= 0) {
    return { text: `${pct}% above 7d baseline (${baseline.toFixed(1)}/d)`, up: true };
  }
  return { text: `${Math.abs(pct)}% below 7d baseline (${baseline.toFixed(1)}/d)`, up: false };
}

interface StatRowProps {
  icon: typeof Database;
  label: string;
  value: React.ReactNode;
  mono?: boolean;
  dim?: boolean;
}

function StatRow({ icon: Icon, label, value, mono, dim }: StatRowProps) {
  return (
    <div className="flex items-baseline justify-between gap-3 border-b border-border/40 py-1.5 last:border-b-0">
      <span className="flex items-center gap-1.5 font-sans text-[11px] text-muted-foreground">
        <Icon className="h-3 w-3" />
        {label}
      </span>
      <span
        className={cn(
          "text-right text-xs",
          mono ? "font-mono tabular-nums" : "font-sans",
          dim ? "text-muted-foreground" : "text-foreground",
        )}
      >
        {value}
      </span>
    </div>
  );
}

function Section({
  title,
  meta,
  children,
}: {
  title: string;
  meta?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section>
      <div className="mb-1.5 flex items-baseline justify-between gap-3">
        <h3 className="font-sans text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground/80">
          {title}
        </h3>
        {meta ? (
          <span className="font-sans text-[10px] text-muted-foreground/70">
            {meta}
          </span>
        ) : null}
      </div>
      {children}
    </section>
  );
}

export function ProfileDetail({
  install,
  onClose,
  onLaunch,
  resolveName,
}: ProfileDetailProps) {
  const [stats, setStats] = useState<ProfileStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadedAt, setLoadedAt] = useState<number>(0);
  const [, setTick] = useState(0);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setStats(null);
    api
      .getProfileStats(install.id)
      .then((s) => {
        if (alive) {
          setStats(s);
          setLoadedAt(Date.now());
        }
      })
      .catch(() => {
        if (alive) setStats(null);
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [install.id]);

  // Tick once a second so the "Updated Xs ago" line breathes.
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(id);
  }, []);

  const todayPlan = stats ? planInference(stats.tokens_today) : null;
  const pace = stats
    ? paceVsBaseline(stats.code_sessions_today, stats.code_sessions_per_day_baseline)
    : null;

  return (
    <div className="sheet-slide flex h-full flex-col">
      {/* Header — codexbar's provider card pattern */}
      <header
        className={cn(
          "border-b px-4 py-3.5",
          install.is_running && "bg-primary/4",
        )}
      >
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 font-sans text-[10px] uppercase tracking-[0.14em] text-muted-foreground">
              <Monitor className="h-3 w-3" />
              <span>
                {install.kind === "default" ? "Default Desktop" : "Profile"}
              </span>
              {install.is_running ? (
                <span className="inline-flex items-center gap-1 rounded-full bg-primary/15 px-1.5 py-0.5 text-[9px] tracking-wider text-primary">
                  <span className="relative inline-flex h-1.5 w-1.5">
                    <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-primary/60" />
                    <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-primary" />
                  </span>
                  live
                </span>
              ) : null}
            </div>
            <div className="mt-0.5 flex items-baseline justify-between gap-2">
              <h2 className="truncate font-display text-2xl leading-tight tracking-tight">
                {install.name}
              </h2>
              {todayPlan ? (
                <span
                  className={cn(
                    "rounded-full px-2 py-0.5 font-sans text-[10px] uppercase tracking-wider",
                    todayPlan.tone === "high"
                      ? "bg-primary/15 text-primary"
                      : "bg-muted text-muted-foreground",
                  )}
                >
                  {todayPlan.label}
                </span>
              ) : null}
            </div>
            {loadedAt > 0 ? (
              <div className="mt-1 font-sans text-[10px] text-muted-foreground/70">
                {install.is_running ? "Active · " : ""}Updated{" "}
                {formatRelativeTime(loadedAt)}
              </div>
            ) : null}
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            aria-label="Close details"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      </header>

      <div className="scrollbar-thin flex-1 space-y-4 overflow-y-auto px-4 py-3">
        {loading && !stats ? (
          <div className="flex h-40 items-center justify-center gap-2 text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span className="font-sans text-xs">Reading profile state…</span>
          </div>
        ) : !stats ? (
          <p className="font-sans text-xs text-muted-foreground">
            Could not read this profile's stats.
          </p>
        ) : (
          <>
            {/* Usage bars — codexbar's hero section */}
            <div className="space-y-3.5">
              <UsageBar
                title="Today"
                meta={
                  stats.tokens_today_date && stats.tokens_today_date !== todayISO()
                    ? `stale · ${stats.tokens_today_date}`
                    : undefined
                }
                value={stats.tokens_today}
                /* Anthropic Max users see ~1-2M tokens/day comfortably;
                 * use 1M as the visual "full bar" reference, capped. */
                scale={1_000_000}
                label={
                  stats.tokens_today > 0
                    ? `${formatNumber(stats.tokens_today)} tokens`
                    : "No tokens recorded today"
                }
                trailing={
                  stats.tokens_today >= 1_000_000
                    ? "1M+"
                    : `${Math.round((stats.tokens_today / 1_000_000) * 100)}% of 1M`
                }
                tone={stats.tokens_today >= 1_000_000 ? "high" : "default"}
              />
              <UsageBar
                title="Last 5 hours"
                value={stats.code_sessions_last_5h}
                /* 5 sessions in 5h ≈ heavy burst — use that as full-bar
                 * reference; very honest because we can see the actual
                 * count. */
                scale={5}
                label={`${stats.code_sessions_last_5h} code session${stats.code_sessions_last_5h === 1 ? "" : "s"}`}
                trailing="rolling window"
              />
              <UsageBar
                title="Last 7 days"
                meta={stats.top_model_last_7d ?? undefined}
                value={stats.code_sessions_last_7d}
                /* ~50 sessions/week is a heavy power user — sets full-bar */
                scale={50}
                label={`${stats.code_sessions_last_7d} code session${stats.code_sessions_last_7d === 1 ? "" : "s"}`}
                trailing={
                  stats.code_sessions_last_30d > 0
                    ? `${stats.code_sessions_last_30d} in 30d`
                    : undefined
                }
                pace={
                  pace ? (
                    <span className="flex items-center gap-1">
                      {pace.up ? (
                        <TrendingUp className="h-2.5 w-2.5" />
                      ) : (
                        <TrendingDown className="h-2.5 w-2.5" />
                      )}
                      {pace.text}
                    </span>
                  ) : null
                }
              />
            </div>

            <div className="h-px bg-border/60" />

            {/* Identity cards */}
            <Section
              title="Accounts in this profile"
              meta={
                stats.identities.length > 0
                  ? `${stats.identities.length} ${
                      stats.identities.length === 1 ? "account" : "accounts"
                    }`
                  : undefined
              }
            >
              {stats.identities.length === 0 ? (
                <p className="font-sans text-[11px] text-muted-foreground/70">
                  No Anthropic identity has used this profile yet.
                </p>
              ) : (
                <ul className="space-y-1.5">
                  {stats.identities.map((id) => (
                    <li
                      key={id.account_id}
                      className={cn(
                        "rounded-md border px-2.5 py-2",
                        id.is_owner
                          ? "border-primary/30 bg-primary/5"
                          : "border-border/60 bg-background/40",
                      )}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex min-w-0 items-center gap-1.5">
                          <User
                            className={cn(
                              "h-3 w-3 shrink-0",
                              id.is_owner ? "text-primary" : "text-muted-foreground",
                            )}
                          />
                          <span
                            className={cn(
                              "truncate font-sans text-xs",
                              id.account_name
                                ? "text-foreground"
                                : "italic text-muted-foreground",
                            )}
                          >
                            {id.account_name ?? "Unnamed account"}
                          </span>
                        </div>
                        {id.is_owner ? (
                          <span className="rounded-full bg-primary/15 px-1.5 py-0.5 font-sans text-[9px] uppercase tracking-wider text-primary">
                            owner
                          </span>
                        ) : (
                          <span className="rounded-full bg-muted px-1.5 py-0.5 font-sans text-[9px] uppercase tracking-wider text-muted-foreground">
                            co-user
                          </span>
                        )}
                      </div>
                      {id.email_address ? (
                        <div className="mt-0.5 flex items-center gap-1 font-mono text-[10px] text-foreground/80">
                          <AtSign className="h-2.5 w-2.5 text-muted-foreground" />
                          <span className="truncate">{id.email_address}</span>
                        </div>
                      ) : null}
                      <div className="mt-1 flex gap-3 font-mono text-[10px] text-muted-foreground">
                        <span title={id.account_id}>
                          acct {id.account_id.slice(0, 8)}
                        </span>
                        {id.agent_session_count > 0 ? (
                          <span>
                            {id.agent_session_count} agent run
                            {id.agent_session_count === 1 ? "" : "s"}
                          </span>
                        ) : null}
                        {id.last_activity_ms ? (
                          <span>{formatRelativeTime(id.last_activity_ms)}</span>
                        ) : null}
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </Section>

            <Section
              title="Storage"
              meta={formatBytes(stats.disk_bytes)}
            >
              <div className="rounded-md bg-muted/30 px-3 py-1">
                <StatRow
                  icon={MessagesSquare}
                  label="Code panel"
                  value={formatBytes(stats.code_panel_bytes)}
                  mono
                  dim={!stats.code_panel_bytes}
                />
                <StatRow
                  icon={Sparkles}
                  label="Cowork agent"
                  value={formatBytes(stats.cowork_agent_bytes)}
                  mono
                  dim={!stats.cowork_agent_bytes}
                />
              </div>
            </Section>

            <Section title="Content">
              <div className="rounded-md bg-muted/30 px-3 py-1">
                <StatRow
                  icon={Blocks}
                  label="Extensions"
                  value={stats.extension_count}
                  mono
                />
                <StatRow
                  icon={Boxes}
                  label="MCP servers"
                  value={stats.mcp_server_count}
                  mono
                />
                <StatRow
                  icon={Hammer}
                  label="Cowork skills"
                  value={stats.cowork_skill_count}
                  mono
                />
                <StatRow
                  icon={Server}
                  label="SSH remotes"
                  value={stats.ssh_remote_count}
                  mono
                  dim={stats.ssh_remote_count === 0}
                />
              </div>
            </Section>

            <Section
              title="Recent projects"
              meta={
                stats.code_recent_cwds.length > 0
                  ? `${stats.code_recent_cwds.length} cwds`
                  : undefined
              }
            >
              {stats.code_recent_cwds.length === 0 ? (
                <p className="font-sans text-[11px] text-muted-foreground/70">
                  No recent code sessions.
                </p>
              ) : (
                <ul className="space-y-0.5 rounded-md bg-muted/30 px-3 py-2 font-mono text-[11px]">
                  {stats.code_recent_cwds.slice(0, 6).map((cwd) => (
                    <li
                      key={cwd}
                      className="truncate text-foreground/80"
                      title={cwd}
                    >
                      <span className="text-muted-foreground/60">›</span>{" "}
                      {tildify(cwd)}
                    </li>
                  ))}
                </ul>
              )}
            </Section>

            <Section title="Sharing">
              <div className="rounded-md bg-muted/30 px-3 py-1">
                <StatRow
                  icon={Hash}
                  label="Workspace link"
                  value={stats.link_group ?? "independent"}
                  mono
                  dim={!stats.link_group}
                />
                {stats.shared_with.length > 0 ? (
                  <div className="py-2">
                    <div className="mb-1 font-sans text-[11px] text-muted-foreground">
                      Shared with
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {stats.shared_with.map((id) => (
                        <span
                          key={id}
                          className="rounded-full bg-primary/10 px-2 py-0.5 font-mono text-[10px] text-primary"
                        >
                          {resolveName(id) ?? id.slice(0, 6)}
                        </span>
                      ))}
                    </div>
                  </div>
                ) : null}
              </div>
            </Section>

            <Section title="Machine">
              <div className="rounded-md bg-muted/30 px-3 py-1">
                <StatRow
                  icon={Cpu}
                  label="Device id"
                  value={stats.device_id?.slice(0, 8) ?? "—"}
                  mono
                  dim={!stats.device_id}
                />
                <StatRow
                  icon={Network}
                  label="Org"
                  value={stats.org_id?.slice(0, 8) ?? "—"}
                  mono
                  dim={!stats.org_id}
                />
                <StatRow
                  icon={Calendar}
                  label="Created"
                  value={formatDate(stats.created_at_ms)}
                />
              </div>
            </Section>

            {/* Quick actions — codexbar's icon+label row at the bottom */}
            <div className="space-y-0.5 border-t pt-3">
              <Button
                size="sm"
                onClick={() => onLaunch(install)}
                disabled={install.is_running}
                className="h-8 w-full justify-start gap-2 rounded-md font-sans text-xs"
              >
                <Play className="h-3 w-3" />
                {install.is_running
                  ? "Already running — bring window forward"
                  : "Launch this profile"}
              </Button>
              <button
                type="button"
                onClick={() => {
                  // Cmd+click in Finder reveals the folder.
                  const url = `file://${encodeURI(stats.data_dir)}`;
                  window.open(url, "_blank");
                }}
                className="flex h-8 w-full items-center gap-2 rounded-md px-3 font-sans text-xs text-foreground/80 transition-colors hover:bg-muted hover:text-foreground"
              >
                <Folder className="h-3 w-3" />
                Open data folder
              </button>
              <button
                type="button"
                onClick={() => navigator.clipboard?.writeText(stats.data_dir)}
                className="flex h-8 w-full items-center gap-2 rounded-md px-3 font-sans text-xs text-foreground/80 transition-colors hover:bg-muted hover:text-foreground"
              >
                <HardDrive className="h-3 w-3" />
                Copy data dir path
              </button>
            </div>

            <div className="break-all border-t pt-3 font-mono text-[10px] text-muted-foreground/70">
              {tildify(stats.data_dir)}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

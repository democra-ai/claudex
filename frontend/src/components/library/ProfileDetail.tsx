import { useEffect, useState } from "react";
import {
  AtSign,
  Calendar,
  Cpu,
  Database,
  Gauge,
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
  User,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { DesktopInstall, ProfileStats } from "@/types";

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

/** Today's date in YYYY-MM-DD form, for comparing against tokens_today_date
 *  to tell stale-from-yesterday data apart from "no usage yet today". */
function todayISO(): string {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
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
    <div className="flex items-baseline justify-between gap-3 border-b border-border/40 py-2 last:border-b-0">
      <span className="flex items-center gap-1.5 font-sans text-[11px] uppercase tracking-[0.12em] text-muted-foreground">
        <Icon className="h-3 w-3" />
        {label}
      </span>
      <span
        className={cn(
          "text-right text-xs",
          mono ? "font-mono" : "font-sans",
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
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section>
      <h3 className="mb-1 font-sans text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground/80">
        {title}
      </h3>
      <div className="rounded-md bg-muted/30 px-3 py-1">{children}</div>
    </section>
  );
}

/** "Hero number" tile — used in the Today section. Two on a row. */
function TileNum({
  icon: Icon,
  label,
  value,
  hint,
}: {
  icon: typeof Database;
  label: string;
  value: React.ReactNode;
  hint?: string;
}) {
  return (
    <div className="rounded-md bg-muted/30 px-3 py-2">
      <div className="flex items-center gap-1.5 font-sans text-[10px] uppercase tracking-[0.12em] text-muted-foreground">
        <Icon className="h-3 w-3" />
        {label}
      </div>
      <div className="mt-1 font-display text-xl leading-tight tabular-nums">
        {value}
      </div>
      {hint ? (
        <div className="mt-0.5 font-mono text-[9px] text-muted-foreground/70">
          {hint}
        </div>
      ) : null}
    </div>
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

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setStats(null);
    api
      .getProfileStats(install.id)
      .then((s) => {
        if (alive) setStats(s);
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

  return (
    <div className="sheet-slide flex h-full flex-col">
      {/* Hero header */}
      <header className="border-b px-4 py-3.5">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5 font-sans text-[10px] uppercase tracking-[0.14em] text-muted-foreground">
              <Monitor className="h-3 w-3" />
              {install.kind === "default" ? "Default Desktop" : "Profile"}
            </div>
            <h2 className="mt-0.5 font-display text-2xl leading-tight tracking-tight">
              {install.name}
            </h2>
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

        <Button
          size="sm"
          onClick={() => onLaunch(install)}
          className="mt-3 h-8 w-full gap-1.5 rounded-md font-sans text-xs"
        >
          <Play className="h-3 w-3" />
          Launch this profile
        </Button>
      </header>

      <div className="scrollbar-thin flex-1 space-y-3 overflow-y-auto px-4 py-3">
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
            {/* Today — usage headline, the codexbar-style top numbers */}
            <Section title="Today">
              <div className="grid grid-cols-2 gap-1.5 py-1">
                <TileNum
                  icon={Gauge}
                  label="Tokens"
                  value={
                    stats.tokens_today > 0
                      ? formatNumber(stats.tokens_today)
                      : "—"
                  }
                  hint={
                    stats.tokens_today_date && stats.tokens_today_date !== todayISO()
                      ? `from ${stats.tokens_today_date}`
                      : stats.tokens_today_date
                      ? "today"
                      : undefined
                  }
                />
                <TileNum
                  icon={MessagesSquare}
                  label="Code sessions"
                  value={formatNumber(stats.code_session_count)}
                />
                <TileNum
                  icon={Sparkles}
                  label="Cowork runs"
                  value={formatNumber(stats.cowork_session_count)}
                />
                <TileNum
                  icon={Calendar}
                  label="Last active"
                  value={
                    <span className="font-sans text-base">
                      {formatRelativeTime(stats.last_activity_ms)}
                    </span>
                  }
                />
              </div>
            </Section>

            {/* Identities — multi-account aware */}
            <Section title="Accounts in this profile">
              {stats.identities.length === 0 ? (
                <p className="py-2 font-sans text-[11px] text-muted-foreground/70">
                  No Anthropic identity has used this profile yet.
                </p>
              ) : (
                <ul className="space-y-1.5 py-1.5">
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
                                : "text-muted-foreground italic",
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
                          <span>{id.agent_session_count} agent run{id.agent_session_count === 1 ? "" : "s"}</span>
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

            <Section title="Storage">
              <StatRow
                icon={HardDrive}
                label="Total"
                value={formatBytes(stats.disk_bytes)}
                mono
              />
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
            </Section>

            <Section title="Recent projects">
              {stats.code_recent_cwds.length === 0 ? (
                <p className="py-2 font-sans text-[11px] text-muted-foreground/70">
                  No recent code sessions.
                </p>
              ) : (
                <ul className="space-y-0.5 py-1.5 pl-1 font-mono text-[11px]">
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

            <Section title="Content">
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
            </Section>

            <Section title="Sharing">
              <StatRow
                icon={Hash}
                label="Workspace link"
                value={stats.link_group ?? "independent"}
                mono
                dim={!stats.link_group}
              />
              {stats.shared_with.length > 0 ? (
                <div className="py-2">
                  <div className="mb-1 font-sans text-[11px] uppercase tracking-[0.12em] text-muted-foreground">
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
            </Section>

            <Section title="Machine">
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
            </Section>

            <div className="break-all border-t pt-3 font-mono text-[10px] text-muted-foreground/70">
              {tildify(stats.data_dir)}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

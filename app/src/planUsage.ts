// Plan quotas, separate from usage.ts (local UI click telemetry).
export const CODEX_STALE_SECS = 1800;
export type Provider = "claude" | "codex";
export type PlanLimit = {
  id: string; bucket: string; bucketLabel: string; label: string;
  percent: number; severity: string; resets_at: number | null;
};
export type PlanUsage = {
  provider: Provider; state: string; source: string;
  fetched_at: number | null; reason?: string | null; limits: PlanLimit[];
};
export type ClaudeUsage = {
  source: string; fetched_at: number;
  limits: { kind: string; label: string; percent: number; severity: string; resets_at: number | null }[];
};
export type CodexUsage = {
  state: string; source: string; fetched_at: number | null; reason: string | null;
  limits: { id: string; bucket_id: string; bucket_label: string; window_role: string;
    window_minutes: number | null; percent: number; severity: string; resets_at: number | null }[];
};
export const providerName = (p: Provider) => p === "claude" ? "Claude" : "Codex";
export const checking = (provider: Provider): PlanUsage => ({ provider, state: "checking", source: "unavailable", fetched_at: null, limits: [] });
export function windowLabel(minutes: number | null, role: string): string {
  if (!minutes || minutes <= 0) return role === "primary" ? "Primary" : "Secondary";
  if (minutes % 1440 === 0) return `${minutes / 1440}d`;
  if (minutes % 60 === 0) return `${minutes / 60}h`;
  return `${minutes}m`;
}
export function adaptClaude(info: ClaudeUsage): PlanUsage {
  return { provider: "claude", state: !info.limits.length || info.source === "unavailable" ? "unavailable"
    : info.source === "oauth" ? "ready" : "stale", source: info.source,
    fetched_at: info.source === "unavailable" ? null : info.fetched_at,
    limits: info.limits.map(l => ({ ...l, id: `${l.kind}:${l.label}`, bucket: "claude", bucketLabel: "Claude",
      severity: l.severity === "normal" || l.severity === "warning" ? l.severity : "over",
      label: l.kind === "session" ? "5h" : l.kind === "weekly_all" ? "7d" : `${l.label} 7d` })) };
}
export function adaptCodex(info: CodexUsage): PlanUsage {
  return { ...info, provider: "codex", limits: info.limits.map(l => ({ ...l, bucket: l.bucket_id,
    bucketLabel: l.bucket_label, label: windowLabel(l.window_minutes, l.window_role) })) };
}
export function viewUsage(info: PlanUsage, now: number): PlanUsage {
  if (info.provider !== "codex" || info.fetched_at === null) return info;
  const age = now - info.fetched_at;
  if (age < 0 || age > CODEX_STALE_SECS) return { ...info, state: "unavailable", reason: "expired", limits: [] };
  if (info.limits.some(l => expired(info, l, now))) return { ...info, state: "stale" };
  return info;
}
export const expired = (info: PlanUsage, limit: PlanLimit, now: number) =>
  info.provider === "codex" && limit.resets_at !== null && limit.resets_at <= now;
export function summaryLimit(info: PlanUsage, now: number): PlanLimit | undefined {
  return info.limits.filter(l => !expired(info, l, now) && (info.provider === "claude" || l.bucket === "codex"))
    .reduce<PlanLimit | undefined>((a, b) => !a || b.percent > a.percent ? b : a, undefined);
}
// Display order never depends on percentages or the provider's response order.
// Main windows precede model/bucket-specific windows; shortest duration first.
export function stripLimits(info: PlanUsage, now: number): PlanLimit[] {
  const duration = (label: string) => {
    const match = /^(\d+)([mhd])$/.exec(label);
    return match ? Number(match[1]) * ({ m: 1, h: 60, d: 1440 }[match[2]] ?? 1) : Infinity;
  };
  const main = (l: PlanLimit) => l.bucket === info.provider && Number.isFinite(duration(l.label));
  return info.limits.filter(l => l.resets_at === null || l.resets_at > now).sort((a, b) =>
    Number(main(b)) - Number(main(a)) || a.bucket.localeCompare(b.bucket) ||
    (duration(a.label) - duration(b.label) || 0) || a.label.localeCompare(b.label) || a.id.localeCompare(b.id));
}
export const stateLabel = (info: PlanUsage) => ({ checking: "checking...", signed_out: "sign in",
  unsupported_auth: "plan unavailable", missing_cli: "CLI missing", stale: "stale" }[info.state]
  ?? (info.state === "ready" && info.limits.length ? "details" : "unavailable"));
export function detailMessage(info: PlanUsage): string {
  switch (info.state) {
    case "checking": return "Checking usage...";
    case "signed_out": return "Not signed in. Sign in through the Codex CLI, then return here.";
    case "unsupported_auth": return "This Codex login does not provide ChatGPT plan usage to this meter.";
    case "missing_cli": return "Codex CLI was not found.";
    case "stale": return info.source === "statusline" ? "Statusline snapshot."
      : info.source === "cached" ? "Could not refresh. Showing the last reading." : "Awaiting updated windows.";
    case "unavailable": return info.reason === "unsupported_protocol" ? "This Codex version cannot provide plan usage."
      : info.reason === "no_limits" ? "No plan-limit windows were reported." : "Could not read usage.";
    default: return "";
  }
}

export const compactUsage = (info: PlanUsage[]) => info.filter(i => !(i.provider === "codex" && i.state === "missing_cli"));

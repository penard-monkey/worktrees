// The policy presets as the page reads them (`policy_preset_status`, core's
// `policy_presets`): types and words only. The VALUES of each preset come from
// the host's answer (`presets`), never from a copy here, so a later release
// that changes Balanced changes what this page shows with it.
import type { Origin } from "./EnvLine";

export type PresetValues = {
  /** Not `codex_permissions`: that name is the blob's mirror key. */
  codex_access: string;
  main_guard: boolean;
  pi_trust: string;
  cross_project: string;
  quota_gate: boolean;
  quota_weekly_pct: number | null;
  max_live_agents: number | null;
};
export type PresetKey = keyof PresetValues;

export type PolicyPreset = {
  name: string;
  label: string;
  summary: string;
  values: PresetValues;
  mutations_default: boolean;
  pair_scope: "observe" | "operate";
};

export type PresetStatus = {
  /** The preset this host is on, or `custom`. */
  name: string;
  applied: { name: string; version: number } | null;
  version: number;
  stale: boolean;
  /** Keys whose value is not `name`'s. */
  modified: PresetKey[];
  matches: string[];
  current: PresetValues;
  presets: PolicyPreset[];
  /** What decided the two host-policy values (§5.4). */
  sources: Partial<Record<PresetKey, Origin>>;
};

/** Every key, in the order the host sets them and the Compare table lists them. */
export const PRESET_KEYS: PresetKey[] = ["codex_access", "main_guard", "pi_trust", "cross_project", "quota_gate", "quota_weekly_pct", "max_live_agents"];

export const PRESET_KEY_LABEL: Record<PresetKey, string> = {
  codex_access: "Codex permissions",
  main_guard: "Claude in (main)",
  pi_trust: "pi project trust",
  cross_project: "Other projects",
  quota_gate: "Plan limits",
  quota_weekly_pct: "Weekly threshold",
  max_live_agents: "Live agents",
};

/** The section on Agent policy each key is changed in. */
export const PRESET_KEY_SECTION: Record<PresetKey, string> = {
  codex_access: "codex-permissions",
  main_guard: "main-guard",
  pi_trust: "pi-trust",
  cross_project: "cross-project",
  quota_gate: "plan-limits",
  quota_weekly_pct: "plan-threshold",
  max_live_agents: "limits",
};

/** A value in the words its own control uses. */
export function presetWords(key: PresetKey, v: PresetValues): string {
  switch (key) {
    case "codex_access": return ({ ask: "Ask", "auto-review": "Auto-review", full: "Full access" } as Record<string, string>)[v.codex_access] ?? v.codex_access;
    case "main_guard": return v.main_guard ? "Guarded" : "Not guarded";
    case "pi_trust": return v.pi_trust === "ask" ? "Let pi ask" : "Skip repo resources";
    case "cross_project": return ({ off: "Off", read: "Read", full: "Full" } as Record<string, string>)[v.cross_project] ?? v.cross_project;
    case "quota_gate": return v.quota_gate ? "Stop on a nearly spent plan" : "No check";
    case "quota_weekly_pct": return !v.quota_gate ? "—" : v.quota_weekly_pct === null ? "The provider's warning" : `${v.quota_weekly_pct}% used`;
    case "max_live_agents": return v.max_live_agents === null ? "This machine's default" : v.max_live_agents === 0 ? "No cap" : `${v.max_live_agents}`;
  }
}

/** Does applying `p` set Codex to Full access? Then Apply asks first, as the
 *  Full access button does (§9 Q4, by the plan's default). */
export const setsFullAccess = (p: PolicyPreset | undefined) => p?.values.codex_access === "full";

/** The two client-side defaults a preset suggests (§5.2's last rows): the
 *  "Allow create and close tools" box and the scope a pairing starts on.
 *  Balanced's, while the status is unknown or Custom. */
export function presetDefaults(st: PresetStatus | null | undefined): { mutations: boolean; pairScope: "observe" | "operate" } {
  const p = st?.presets.find((x) => x.name === st.name);
  return { mutations: p?.mutations_default ?? true, pairScope: p?.pair_scope ?? "observe" };
}

// Settings → Agent policy (settings-reorg §3.2): the one page that answers
// "how much may my agents do", which used to have five answers in five places.
// Every value here is the HOST's (host-policy.json, config.toml, the user
// JSON), shared by every window and applied to every lane it launches.
import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { invoke, notGranted } from "../ipc";
import type { CodexPermissions, PiProjectTrust } from "../settings";
import { SECTION } from "../settingsMap";
import type { GuidanceStatus } from "../GuidancePanel";
import type { CrossProjectStatus } from "../CrossProjectPanel";
import {
  PRESET_KEYS, PRESET_KEY_LABEL, PRESET_KEY_SECTION, presetWords, setsFullAccess, type PresetKey, type PresetStatus,
} from "../presets";
import { GuardSection } from "../GuidancePanel";
import { PiTrustSection, usePiStatus } from "../PiPanel";
import { KimiTrustSection, useKimiStatus } from "../KimiPanel";
import { CrossProjectSection } from "../CrossProjectPanel";
import { QuotaSection, QuotaThresholdSection, useQuota } from "../QuotaPanel";
import { SlotsSection } from "../SlotsPanel";
import { LimitsSection } from "../LimitsPanel";
import { useArmed } from "../useArmed";
import { EnvLine, type Origin } from "../EnvLine";
import { Page, Sec, useSheet } from "./kit";

/** The line a Full access click must be confirmed against (§6 item 3): the one
 *  control that widens what an agent may do on the machine gets the friction
 *  the other two do not need. */
export const FULL_ACCESS_CONFIRM = "Full access: every Codex lane this computer launches runs with no prompts and no sandbox.";

/** Codex → Permissions. HOST policy, not a setting of this client: the host
 *  keeps it (`host-policy.json`) and decides every lane it launches, so the
 *  control renders what the host answers and only a click here changes it. A
 *  refusal rolls the control back and is reported through `fail`, so it is
 *  never mistaken for a change that took.
 *
 *  Every set is followed by a fresh read, and only the newest read may land
 *  (`seq`): two quick clicks can reach a server out of order, and the control
 *  must end on what the host STORED, not on whichever reply came back last.
 *
 *  Once a set has succeeded, each read that lands is also copied into
 *  `Settings.codex_permissions` for an older build to find (`settings.ts`).
 *  Not before: until then the read may be env or user config speaking. */
function CodexPermissionsSection({ onFail, onMirror }: { onFail: (e: unknown) => void; onMirror: (m: CodexPermissions) => void }) {
  const [mode, setMode] = useState<CodexPermissions | null>(null);
  /** What decided `mode` (§6 item 4); null from a host older than `detail`. */
  const [origin, setOrigin] = useState<Origin | null>(null);
  const seq = useRef(0);
  const held = useRef(false);
  const mirror = useRef(onMirror);
  mirror.current = onMirror;
  const arm = useArmed<"full">();
  const readHost = useCallback(() => {
    const n = ++seq.current;
    // `detail` answers `{value, source, env}`; an older host ignores it and
    // answers the bare string.
    invoke<CodexPermissions | ({ value: CodexPermissions } & Origin)>("get_codex_permissions", { detail: true })
      .then((r) => {
        if (seq.current !== n) return;
        const m = typeof r === "string" ? r : r.value;
        setMode(m);
        setOrigin(typeof r === "string" ? null : { source: r.source, env: r.env });
        if (held.current) mirror.current(m);
      })
      .catch((e) => onFail(`Codex permissions: ${String(e)}`));
  }, [onFail]);
  useEffect(() => {
    readHost();
    return () => { seq.current++; };
  }, [readHost]);
  const set = (next: CodexPermissions) => {
    const prev = mode;
    const n = ++seq.current;
    setMode(next);
    invoke("set_codex_permissions", { mode: next }).then(() => { held.current = true; readHost(); }, (e) => {
      if (seq.current === n) setMode(prev);
      onFail(`Codex permissions: ${String(e)}`);
      readHost();
    });
  };
  const pick = (next: CodexPermissions) => {
    if (next === mode) return;
    if (next === "full") { arm.confirm("full", () => set("full")); return; }
    arm.disarm();
    set(next);
  };
  return (
    <section className="setting">
      <label>Codex permissions</label>
      <div className="seg seg-plain" data-testid="codex-permissions">
        {([
          ["ask", "Ask"],
          ["auto-review", "Auto-review"],
          ["full", "Full access"],
        ] as const).map(([m, label]) => (
          <button key={m} className={(mode === m ? "on" : "") + (m === "full" && arm.armed ? " armed" : "")} disabled={mode === null}
            data-testid={`codex-permissions-${m}`} onClick={() => pick(m)}>
            {m === "full" && arm.armed ? "Confirm full access" : label}
          </button>
        ))}
      </div>
      {arm.armed && <div className="hint confirm-line" data-testid="codex-full-confirm">{FULL_ACCESS_CONFIRM} Click again to confirm.</div>}
      {/* All three lines, always: the choice is only honest if you can read
          what you are NOT picking. The current one is bold. */}
      {([
        ["ask", "Ask", "Codex asks before running commands."],
        ["auto-review", "Auto-review", "a reviewer approves or refuses each request; commands run sandboxed to this worktree plus the repo's git data (hooks and config included, so commits work) and the network."],
        ["full", "Full access", "no prompts and no sandbox."],
      ] as const).map(([m, label, line]) => (
        <div className="hint" key={m}>
          {mode === m ? <b>{label}:</b> : `${label}:`} {line}
        </div>
      ))}
      <EnvLine origin={origin} outranks="control" />
      <div className="hint">Applies to the next Codex launch; a session already running keeps what it started with.</div>
    </section>
  );
}

// ── the preset (§5) ─────────────────────────────────────────────────────────

/** The one call that applies a preset — the user's act alone: every reference
 *  to it is a click (hostpolicy-check.mjs). `keys`: only those (a control's
 *  "back to <preset>"); `name: "custom"` with none forgets the name. */
function applyPreset(name: string, keys: PresetKey[] | null): Promise<PresetStatus> {
  return invoke<PresetStatus>("apply_policy_preset", { name, keys });
}

/** The page's one read of `policy_preset_status`, shared by the Preset
 *  section and every "back to <preset>" line. */
function usePresetStatus(onReport: (m: string) => void) {
  const [st, setSt] = useState<PresetStatus | null>(null);
  const read = useCallback(() => {
    invoke<PresetStatus>("policy_preset_status").then(setSt).catch((e) => onReport(`policy_preset_status: ${String(e)}`));
  }, [onReport]);
  useEffect(() => { read(); }, [read]);
  return { st, setSt, read };
}

/** Settings → Agent policy → Preset: one word for the values below. Applying
 *  rewrites them through their own setters; nothing asks on leaving a preset
 *  — the marker says what moved, each moved control offers its way back, and
 *  Compare shows all three, so the choice is honest about what is not picked. */
function PresetSection({ st, onApplied, onFail }: {
  st: PresetStatus | null;
  onApplied: (res: PresetStatus, keys: PresetKey[] | null) => void;
  onFail: (e: unknown) => void;
}) {
  const { goto } = useSheet();
  const arm = useArmed<string>();
  const [busy, setBusy] = useState(false);
  const [compare, setCompare] = useState(false);
  const run = async (name: string, keys: PresetKey[] | null) => {
    setBusy(true);
    try { onApplied(await applyPreset(name, keys), keys); }
    catch (e) { onFail(`Policy preset: ${String(e)}`); }
    finally { setBusy(false); }
  };
  const pick = (name: string) => {
    const p = st?.presets.find((x) => x.name === name);
    if (!st || (st.name === name && st.modified.length === 0 && st.applied?.name === name)) return;
    // Full access asks first, here as on its own button (§9 Q4).
    if (setsFullAccess(p) && st.current.codex_access !== "full") { arm.confirm(name, () => void run(name, null)); return; }
    arm.disarm();
    void run(name, null);
  };
  const cur = st?.presets.find((x) => x.name === st.name);
  const armedP = st?.presets.find((x) => x.name === arm.armed);
  return (
    <section className="setting" data-testid="policy-preset">
      <label>Preset</label>
      <div className="seg seg-plain" role="radiogroup" aria-label="Policy preset" data-testid="preset-seg">
        {(st?.presets ?? []).map((p) => (
          <button key={p.name} role="radio" aria-checked={st?.name === p.name} data-preset={p.name}
            className={(st?.name === p.name ? "on" : "") + (arm.armed === p.name ? " armed" : "")}
            disabled={!st || busy} onClick={() => pick(p.name)}>
            {arm.armed === p.name ? `Confirm ${p.label}` : p.label}
          </button>
        ))}
        <button role="radio" aria-checked={st?.name === "custom"} data-preset="custom" className={st?.name === "custom" ? "on" : ""}
          disabled={!st || busy || st.name === "custom" || st.modified.length === 0} title="Keep these values, under no preset's name"
          onClick={() => void run("custom", null)}>Custom</button>
      </div>
      {armedP && <div className="hint confirm-line" data-testid="preset-full-confirm">
        {armedP.label} sets Codex permissions to Full access. {FULL_ACCESS_CONFIRM} Click again to confirm.
      </div>}
      {!st ? <div className="hint">Checking…</div> : <>
        <div className="hint" data-testid="preset-summary">
          {cur ? <><b>{cur.label}:</b> {cur.summary}</> : <><b>Custom:</b> these values match no preset.</>}
        </div>
        {st.modified.length > 0 && cur && (
          <div className="preset-modified" data-testid="preset-modified">
            <span>{cur.label} (modified: </span>
            {st.modified.map((k, i) => (
              <span key={k}>
                {i > 0 && ", "}
                <button type="button" className="mcp-dismiss" onClick={() => goto("policy", PRESET_KEY_SECTION[k])}>{PRESET_KEY_LABEL[k]}</button>
                {st.sources[k]?.source === "env" ? " — set by the environment" : ""}
              </span>
            ))}
            <span>)</span>
            <span className="preset-acts">
              <button type="button" className="ctrl sm" data-testid="preset-reset" disabled={busy}
                onClick={() => (setsFullAccess(cur) && st.current.codex_access !== "full" ? arm.confirm(cur.name, () => void run(cur.name, null)) : void run(cur.name, null))}>
                Reset to {cur.label}
              </button>
              <button type="button" className="ctrl sm" data-testid="preset-keep" disabled={busy} onClick={() => void run("custom", null)}>Keep as custom</button>
            </span>
          </div>
        )}
        {st.stale && <div className="hint">This host applied {cur?.label ?? st.applied?.name} under an older meaning of it; Reset to apply what it means now.</div>}
        <button type="button" className="mcp-dismiss preset-compare-toggle" data-testid="preset-compare" onClick={() => setCompare((c) => !c)}>
          {compare ? "Hide the comparison" : "Compare presets ›"}
        </button>
        {compare && (
          <table className="preset-compare" data-testid="preset-table">
            <thead>
              <tr><th />{st.presets.map((p) => <th key={p.name} className={p.name === st.name ? "cur" : ""}>{p.label}</th>)}</tr>
            </thead>
            <tbody>
              {PRESET_KEYS.map((k) => (
                <tr key={k}>
                  <th scope="row">{PRESET_KEY_LABEL[k]}</th>
                  {st.presets.map((p) => (
                    <td key={p.name} className={p.name === st.name ? "cur" : ""}>
                      {presetWords(k, p.values)}
                      {p.name === st.name && st.modified.includes(k) && <span className="preset-now"> · now {presetWords(k, st.current)}</span>}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <div className="hint">A preset writes this computer's own settings, below; it never changes a window's. Each value stays yours to change after.</div>
      </>}
    </section>
  );
}

/** Under a control whose value is not its preset's: the way back to that one
 *  value (§5.3, PrusaSlicer's back arrow). Drawn only where it can act — a
 *  paired device may not apply a preset. */
function BackToPreset({ k, st, onApplied, onFail }: {
  k: PresetKey;
  st: PresetStatus | null;
  onApplied: (res: PresetStatus, keys: PresetKey[] | null) => void;
  onFail: (e: unknown) => void;
}) {
  const arm = useArmed<"back">();
  const [busy, setBusy] = useState(false);
  const p = st?.presets.find((x) => x.name === st.name);
  // The preset row's own name for its write (settingsMap), not a literal:
  // a paired device may not apply a preset, so it is not offered one here.
  if (!st || !p || !st.modified.includes(k) || notGranted(SECTION["policy-preset"].write ?? "")) return null;
  const restore = async () => {
    setBusy(true);
    try { onApplied(await applyPreset(p.name, [k]), [k]); }
    catch (e) { onFail(`Policy preset: ${String(e)}`); }
    finally { setBusy(false); }
  };
  const full = k === "codex_access" && p.values.codex_access === "full";
  return (
    <div className="preset-back" data-testid={`preset-back-${k}`}>
      <button type="button" className="mcp-dismiss" disabled={busy}
        onClick={() => (full ? arm.confirm("back", () => void restore()) : void restore())}>
        {arm.armed ? `Confirm: ${FULL_ACCESS_CONFIRM}` : `↶ Back to ${p.label} (${presetWords(k, p.values)})`}
      </button>
    </div>
  );
}

export function PolicyPage() {
  const {
    settings, repo, onChange, onReport, onFail, guidance, onGuidanceChanged,
    crossProject, onCrossProjectChanged, crossProjectOfferPending, onSilenceCrossProjectOffer,
  } = useSheet();
  const pi = usePiStatus({ settings, repo: repo || null, onChange, onReport, onFail });
  const kimi = useKimiStatus(repo || null, onReport);
  const quota = useQuota(onReport);
  const [limitsProblem, setLimitsProblem] = useState(false);
  const preset = usePresetStatus(onReport);
  /** Bumped by every preset apply: the sections below re-read what it wrote. */
  const [rev, setRev] = useState(0);
  const onApplied = (res: PresetStatus, keys: PresetKey[] | null) => {
    preset.setSt(res);
    setRev((r) => r + 1);
    quota.reload();
    pi.readStatus();
    invoke<GuidanceStatus>("agent_guidance_status").then(onGuidanceChanged).catch(() => {});
    invoke<CrossProjectStatus>("cross_project_status").then(onCrossProjectChanged).catch(() => {});
    // An older build's copy of the two host policies (settings.ts), written
    // only for what this apply SET — the host now holds those keys.
    const wrote = (k: PresetKey) => keys === null || keys.includes(k);
    if (wrote("codex_access") || wrote("pi_trust")) onChange({
      ...(wrote("codex_access") ? { codex_permissions: res.current.codex_access as CodexPermissions } : null),
      ...(wrote("pi_trust") ? { pi_project_trust: res.current.pi_trust as PiProjectTrust } : null),
    });
  };
  const backTo = (k: PresetKey) => <BackToPreset k={k} st={preset.st} onApplied={onApplied} onFail={onFail} />;
  return <Page page="policy">
    <Sec id="policy-preset">
      <PresetSection st={preset.st} onApplied={onApplied} onFail={onFail} />
    </Sec>

    <Fragment key={rev}>
    <Sec id="codex-permissions">
      {backTo("codex_access")}
      <CodexPermissionsSection onFail={onFail} onMirror={(m) => { if (settings.codex_permissions !== m) onChange({ codex_permissions: m }); }} />
    </Sec>

    <Sec id="main-guard">
      {backTo("main_guard")}
      <GuardSection status={guidance} onStatus={onGuidanceChanged} onReport={onReport} />
    </Sec>

    <Sec id="pi-trust">
      {backTo("pi_trust")}
      <PiTrustSection pi={pi} />
    </Sec>

    <Sec id="kimi-trust">
      <KimiTrustSection status={kimi.status} allow={(r, on) => void kimi.allow(r, on)} />
    </Sec>

    <Sec id="cross-project">
      {backTo("cross_project")}
      <CrossProjectSection status={crossProject} onStatus={onCrossProjectChanged} onReport={onReport}
        offerPending={crossProjectOfferPending} onSilenceOffer={onSilenceCrossProjectOffer} />
    </Sec>

    <Sec id="plan-limits" problem={(quota.s?.problems.length ?? 0) > 0}>
      {backTo("quota_gate")}
      <QuotaSection q={quota} />
    </Sec>

    <Sec id="plan-threshold">
      {backTo("quota_weekly_pct")}
      <QuotaThresholdSection q={quota} />
    </Sec>

    <Sec id="limits" problem={limitsProblem}>
      {backTo("max_live_agents")}
      <LimitsSection onReport={onReport} onProblems={setLimitsProblem} />
    </Sec>

    <Sec id="heavy-jobs">
      <SlotsSection onReport={onReport} />
    </Sec>
    </Fragment>
  </Page>;
}

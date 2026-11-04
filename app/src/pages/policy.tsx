// Settings → Agent policy (settings-reorg §3.2): the one page that answers
// "how much may my agents do", which used to have five answers in five places.
// Every value here is the HOST's (host-policy.json, config.toml, the user
// JSON), shared by every window and applied to every lane it launches.
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "../ipc";
import type { CodexPermissions } from "../settings";
import { GuardSection } from "../GuidancePanel";
import { PiTrustSection, usePiStatus } from "../PiPanel";
import { KimiTrustSection, useKimiStatus } from "../KimiPanel";
import { CrossProjectSection } from "../CrossProjectPanel";
import { QuotaSection, QuotaThresholdSection, useQuota } from "../QuotaPanel";
import { SlotsSection } from "../SlotsPanel";
import { useArmed } from "../useArmed";
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
  const seq = useRef(0);
  const held = useRef(false);
  const mirror = useRef(onMirror);
  mirror.current = onMirror;
  const arm = useArmed<"full">();
  const readHost = useCallback(() => {
    const n = ++seq.current;
    invoke<CodexPermissions>("get_codex_permissions")
      .then((m) => {
        if (seq.current !== n) return;
        setMode(m);
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
      <div className="hint">Applies to the next Codex launch; a session already running keeps what it started with.</div>
    </section>
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
  return <Page page="policy">
    <Sec id="codex-permissions">
      <CodexPermissionsSection onFail={onFail} onMirror={(m) => { if (settings.codex_permissions !== m) onChange({ codex_permissions: m }); }} />
    </Sec>

    <Sec id="main-guard">
      <GuardSection status={guidance} onStatus={onGuidanceChanged} onReport={onReport} />
    </Sec>

    <Sec id="pi-trust">
      <PiTrustSection pi={pi} />
    </Sec>

    <Sec id="kimi-trust">
      <KimiTrustSection status={kimi.status} allow={(r, on) => void kimi.allow(r, on)} />
    </Sec>

    <Sec id="cross-project">
      <CrossProjectSection status={crossProject} onStatus={onCrossProjectChanged} onReport={onReport}
        offerPending={crossProjectOfferPending} onSilenceOffer={onSilenceCrossProjectOffer} />
    </Sec>

    <Sec id="plan-limits" problem={(quota.s?.problems.length ?? 0) > 0}>
      <QuotaSection q={quota} />
    </Sec>

    <Sec id="plan-threshold">
      <QuotaThresholdSection q={quota} />
    </Sec>

    <Sec id="heavy-jobs">
      <SlotsSection onReport={onReport} />
    </Sec>
  </Page>;
}

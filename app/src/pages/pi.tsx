// Settings → pi (pi-harness §5, §8): the Worktrees tools in pi, which pi and
// node a pane gets, pi's models — and a pointer to its trust, which moved to
// Agent policy (settings-reorg §3.2).
import { PiMcpSection } from "../PiMcpPanel";
import { PiInstallSection, PiModelsSection, usePiModels, usePiStatus } from "../PiPanel";
import { presetDefaults } from "../presets";
import { Page, Pointer, Sec, useSheet } from "./kit";

export function PiPage() {
  const { settings, repo, onChange, onReport, onFail, onPiMcpChanged, piMcpOfferPending, onSilencePiMcpOffer, snap } = useSheet();
  const pi = usePiStatus({ settings, repo: repo || null, onChange, onReport, onFail });
  const { models, reload: reloadModels } = usePiModels(onReport);
  const allowed = pi.status?.allowed.length ?? 0;
  return <Page page="pi">
    {/* FIRST: a deep-link target (offers.ts `pi-mcp`), and the sections below
        fill in asynchronously — anything under them is pushed off screen after
        the focus scroll lands (the Codex page's note). */}
    <Sec id="pi-mcp">
      <PiMcpSection onReport={onReport} onStatus={onPiMcpChanged}
        offerPending={piMcpOfferPending} onSilenceOffer={onSilencePiMcpOffer} mutationsDefault={presetDefaults(snap.preset).mutations} />
    </Sec>
    <Sec id="pi-install" problem={!!pi.status?.preflight.problem}>
      <PiInstallSection status={pi.status} />
    </Sec>
    <Sec id="pi-models">
      <PiModelsSection settings={settings} models={models} onChange={onChange} reloadModels={reloadModels} onFail={onFail} />
    </Sec>
    <Sec id="pi-policy">
      <Pointer to="pi-trust" label="Agent policy">
        Project trust: <b>{!pi.status ? "…" : pi.status.trust === "ask" ? "Let pi ask" : "Skip repo resources"}</b>
        {" · "}{allowed === 0 ? "no allowed repos" : `${allowed} allowed repo${allowed === 1 ? "" : "s"}`}
      </Pointer>
    </Sec>
  </Page>;
}

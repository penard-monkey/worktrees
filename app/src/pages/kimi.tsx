// Settings → Kimi: the Worktrees tools in kimi lanes, which kimi a pane gets,
// kimi's models — and a pointer to its folder trust, which moved to Agent
// policy (settings-reorg §3.2).
import { KimiInstallSection, KimiMcpSection, KimiModelsSection, useKimiStatus } from "../KimiPanel";
import { Page, Pointer, Sec, useSheet } from "./kit";

export function KimiPage() {
  const { settings, repo, onChange, onReport, onKimiMcpChanged, kimiMcpOfferPending, onSilenceKimiMcpOffer } = useSheet();
  const kimi = useKimiStatus(repo || null, onReport);
  const allowed = kimi.status?.allowed.length ?? 0;
  return <Page page="kimi">
    {/* FIRST: the `kimi-mcp` offer's deep-link target; the sections below
        fill in asynchronously and would push it off screen. */}
    <Sec id="kimi-mcp">
      <KimiMcpSection onReport={onReport} onStatus={onKimiMcpChanged}
        offerPending={kimiMcpOfferPending} onSilenceOffer={onSilenceKimiMcpOffer} />
    </Sec>
    <Sec id="kimi-install">
      <KimiInstallSection status={kimi.status} />
    </Sec>
    <Sec id="kimi-models">
      <KimiModelsSection settings={settings} status={kimi.status} onChange={onChange} onReport={onReport} />
    </Sec>
    <Sec id="kimi-policy">
      <Pointer to="kimi-trust" label="Agent policy">
        Folder trust: <b>{!kimi.status ? "…" : allowed === 0 ? "kimi asks in every place" : `${allowed} allowed repo${allowed === 1 ? "" : "s"}`}</b>
      </Pointer>
    </Sec>
  </Page>;
}

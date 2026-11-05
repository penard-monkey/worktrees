// Settings → Guidance (agent-guidance §4.5, decision Q8): whether agents get
// it, the skill they get (Expert), and a pointer to the (main) guard, which
// moved to Agent policy (settings-reorg §3.2).
import { GuidanceSection, GuidanceSkillSection } from "../GuidancePanel";
import { Page, Pointer, Sec, useSheet } from "./kit";

export function GuidancePage() {
  const {
    guidance, onGuidanceChanged, onReport, guidanceOfferPending, onSilenceGuidanceOffer,
    guidanceChangeOfferPending, onSilenceGuidanceChangeOffer,
  } = useSheet();
  return <Page page="guidance">
    <Sec id="agent-guidance">
      <GuidanceSection status={guidance} onStatus={onGuidanceChanged} onReport={onReport}
        offerPending={guidanceOfferPending} onSilenceOffer={onSilenceGuidanceOffer} />
    </Sec>
    <Sec id="guidance-policy">
      <Pointer to="main-guard" label="Agent policy">
        Stop Claude branching in (main): <b>{!guidance ? "…" : guidance.settings.guard ? "on" : "off"}</b>
      </Pointer>
    </Sec>
    <Sec id="guidance-skill" problem={!!guidance?.skill_edit?.invalid}>
      <GuidanceSkillSection status={guidance} onStatus={onGuidanceChanged} onReport={onReport}
        changeOfferPending={guidanceChangeOfferPending} onSilenceChangeOffer={onSilenceGuidanceChangeOffer} />
    </Sec>
  </Page>;
}

// Settings → Codex (settings-reorg §3.2): your skills for Codex, the
// Worktrees tools in Codex (and copying servers over from Claude), and a
// pointer to Permissions, which moved to Agent policy.
import { useState } from "react";
import { UserSkillsSection } from "../AgentSetup";
import { CodexMcpSection, type CodexMcpStatus } from "../CodexMcpPanel";
import { Page, Pointer, Sec, useSheet } from "./kit";

export const CODEX_PERMISSION_WORDS = { ask: "Ask", "auto-review": "Auto-review", full: "Full access" } as const;

export function CodexPage() {
  const {
    userSkills, onUserSkillsChanged, skillsOfferPending, onSilenceSkillsOffer, onReport,
    onCodexMcpChanged, codexMcpOfferPending, onSilenceCodexMcpOffer, snap,
  } = useSheet();
  const [mcp, setMcp] = useState<CodexMcpStatus | null>(null);
  return <Page page="codex">
    {/* ORDER IS LOAD-BEARING: both sections below are deep-link targets
        (offers.ts), and the focus effect scrolls once, on the next frame. The
        MCP panel and its migration table fill in ASYNCHRONOUSLY, so anything
        under them is pushed off screen after the scroll landed — measured:
        "Your skills" ended at y=1258 in an 897px window. Skills renders from
        App's state at once, so it goes above. */}
    <Sec id="codex-skills">
      <UserSkillsSection skills={userSkills} onChanged={onUserSkillsChanged}
        offerPending={skillsOfferPending} onSilenceOffer={onSilenceSkillsOffer} onReport={onReport} />
    </Sec>
    <Sec id="codex-mcp" problem={mcp?.state === "stale"}>
      <CodexMcpSection onReport={onReport} onStatus={(s) => { setMcp(s); onCodexMcpChanged(s); }}
        offerPending={codexMcpOfferPending} onSilenceOffer={onSilenceCodexMcpOffer} />
    </Sec>
    <Sec id="codex-policy">
      <Pointer to="codex-permissions" label="Agent policy">
        Permissions: <b>{snap.codexPermissions ? CODEX_PERMISSION_WORDS[snap.codexPermissions] : "…"}</b>
      </Pointer>
    </Sec>
  </Page>;
}

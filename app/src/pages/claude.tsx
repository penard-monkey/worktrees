// Settings → Claude (settings-reorg §3.2): the Worktrees tools in Claude, and
// a pointer to the one Claude policy, the (main) guard, which moved to Agent
// policy.
import { McpSection } from "../McpPanel";
import { Page, Pointer, Sec, useSheet } from "./kit";

export function ClaudePage() {
  const { mcpStatus, repo, mcpOfferPending, onSilenceMcpOffer, onMcpChanged, onReport, guidance } = useSheet();
  const guard = guidance?.settings.guard;
  return <Page page="claude">
    <Sec id="mcp-server">
      <McpSection status={mcpStatus} repo={repo} offerPending={mcpOfferPending}
        onSilenceOffer={onSilenceMcpOffer} onChanged={onMcpChanged} onReport={onReport} />
    </Sec>
    <Sec id="claude-policy">
      <Pointer to="main-guard" label="Agent policy">
        Stop Claude branching in (main): <b>{guard === undefined ? "…" : guard ? "on" : "off"}</b>
      </Pointer>
    </Sec>
  </Page>;
}

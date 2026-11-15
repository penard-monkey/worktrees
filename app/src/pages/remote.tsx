// Settings → Remote (settings-reorg §3.2): the guest's Join with a link
// (group chat start design S4), which engine THIS window drives (the client's own record), and who else may
// reach this computer's engine: the listener, and the devices that use it
// (the host's, console-only). On a window driving a paired host the last two
// say so, which still leaves the page worth opening for the first.
import { useRemote } from "../remoteStore";
import { HostsPanel } from "../HostsPanel";
import { JoinHostSection } from "../JoinHost";
import { groupChatBeta } from "../beta";
import { RemoteAccessSection } from "../RemoteAccessPanel";
import { DevicesSection } from "../DevicesPanel";
import { GuestsSection } from "../GuestsPanel";
import { presetDefaults } from "../presets";
import { Page, Sec, useSheet } from "./kit";

export function RemotePage() {
  const { onReport, onFail, mode, goto, snap, settings, onChange } = useSheet();
  const r = useRemote();
  const listenerProblem = r.status?.state === "unreadable" || r.status?.state === "error" || r.status?.state === "paused"
    || r.alerts.some((a) => a.cat === "remote");
  return <Page page="remote">
    <Sec id="join-host">
      <JoinHostSection groupChat={groupChatBeta(settings)} onReport={onReport} onOpenBeta={() => goto("general", "beta")} />
    </Sec>
    <Sec id="this-window-drives">
      <HostsPanel onReport={onReport} onFail={onFail} manage={mode !== "beginner"} />
    </Sec>
    <Sec id="remote-access" problem={listenerProblem}>
      <RemoteAccessSection onReport={onReport} advertise={mode === "expert" || r.status?.advertise === false} />
    </Sec>
    <Sec id="devices" problem={r.alerts.some((a) => a.cat === "devices")}>
      <DevicesSection onRemote={() => goto("remote", "remote-access")} stepup={mode === "expert"}
        pairScope={presetDefaults(snap.preset).pairScope} />
    </Sec>
    <Sec id="guests">
      <GuestsSection onRemote={() => goto("remote", "remote-access")} />
      {/* *Use these*' ask before a send with no note, and the way back from
          its "Don't ask again" (this window's own setting). */}
      <section className="setting">
        <label className="tier-toggle setting-check">
          <input type="checkbox" checked={settings.use_these_ask_note} data-testid="use-these-ask-setting"
            onChange={(e) => onChange({ use_these_ask_note: e.currentTarget.checked })} />
          Ask before sending a guest’s comments to the agent with no note
        </label>
        <div className="hint">The note is what the agent is asked to do; without one it only reads the comments.</div>
      </section>
    </Sec>
  </Page>;
}

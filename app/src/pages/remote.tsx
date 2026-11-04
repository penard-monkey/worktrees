// Settings → Remote (settings-reorg §3.2): three sections, three homes —
// which engine THIS window drives (the client's own record), and who else may
// reach this computer's engine: the listener, and the devices that use it
// (the host's, console-only). On a window driving a paired host the last two
// say so, which still leaves the page worth opening for the first.
import { useRemote } from "../remoteStore";
import { HostsPanel } from "../HostsPanel";
import { RemoteAccessSection } from "../RemoteAccessPanel";
import { DevicesSection } from "../DevicesPanel";
import { Page, Sec, useSheet } from "./kit";

export function RemotePage() {
  const { onReport, onFail, mode, goto } = useSheet();
  const r = useRemote();
  const listenerProblem = r.status?.state === "unreadable" || r.status?.state === "error" || r.status?.state === "paused"
    || r.alerts.some((a) => a.cat === "remote");
  return <Page page="remote">
    <Sec id="this-window-drives">
      <HostsPanel onReport={onReport} onFail={onFail} manage={mode !== "beginner"} />
    </Sec>
    <Sec id="remote-access" problem={listenerProblem}>
      <RemoteAccessSection onReport={onReport} advertise={mode === "expert" || r.status?.advertise === false} />
    </Sec>
    <Sec id="devices" problem={r.alerts.some((a) => a.cat === "devices")}>
      <DevicesSection onRemote={() => goto("remote", "remote-access")} stepup={mode === "expert"} />
    </Sec>
  </Page>;
}

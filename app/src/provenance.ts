// Who did what to a place, as its declared state records it (Server Edition
// Phase 2, §7.3 and §7.5 — `store.rs`'s `By`, `LaunchedBy`,
// `RemoteInstruction`). The JSON is `authz::Caller`'s spelling of a principal
// on a device. Pure: App.tsx renders what these return.

export type DeviceRef = { kind: "console" } | { kind: "paired"; id: string };

/** A principal on a device. */
export type By = { principal: string; device: DeviceRef };

/** Who launched the place's agent through serve; `cross_project: "off"` when
 *  a device whose scope is a list launched it (its lanes reach no other
 *  project). */
export type LaunchedBy = By & { cross_project?: string };

/** The last instruction-class file a paired device wrote into the place:
 *  `path` is relative to the place, `at` in epoch seconds. */
export type RemoteInstruction = By & { path: string; at: number };

/** What a device is called on screen. Device NAMES are slice 5's registry
 *  (#544): until it lands, the id. */
export function deviceLabel(d: DeviceRef): string {
  if (d.kind === "console") return "this computer";
  return d.id; // slice 5: the registry's name for d.id
}

/** True for a paired device — what the place header speaks about. The
 *  console is the person at this desk; saying so would be noise. */
export function fromDevice(by?: By | null): by is By & { device: { kind: "paired"; id: string } } {
  return by?.device.kind === "paired";
}

/** "2 min ago", in words: this line is read, not scanned. */
export function agoWords(epoch: number, now: number = Date.now() / 1000): string {
  const s = Math.max(0, Math.floor(now - epoch));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

/** "AGENTS.md edited from <device>, 2 min ago" — the file's own name; the
 *  full path is the title's. */
export function instructionLine(i: RemoteInstruction, now?: number): string {
  const name = i.path.split("/").pop() || i.path;
  return `${name} edited from ${deviceLabel(i.device)}, ${agoWords(i.at, now)}`;
}

/** The longer form, for a tooltip. */
export function instructionTitle(i: RemoteInstruction): string {
  const who = i.principal === "owner" ? "" : ` (${i.principal})`;
  return `${i.path} was last written from the paired device ${deviceLabel(i.device)}${who} at ${new Date(i.at * 1000).toLocaleString()}. Review it in Changes before an agent reads it.`;
}

/** "from <device>" beside a live session the device launched, and why its
 *  reach may be narrower than this computer's. */
export function launchedTitle(l: LaunchedBy): string {
  const who = l.principal === "owner" ? "" : ` (${l.principal})`;
  const reach = l.cross_project === "off" ? " Its scope is a list of projects, so this agent reaches no other project." : "";
  return `This agent was launched from the paired device ${deviceLabel(l.device)}${who}.${reach}`;
}

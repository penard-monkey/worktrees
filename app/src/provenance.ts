// Who did what to a place, as its declared state records it (Server Edition
// Phase 2, §7.3 and §7.5 — `store.rs`'s `By`, `LaunchedBy`,
// `RemoteInstruction`). The JSON is `authz::Caller`'s spelling of a principal
// on a device. Pure: App.tsx renders what these return.

/** A paired device's `id` is absent when the host shows this window another
 *  device's stamp: a window that is itself a paired device is told which
 *  device by name only. */
export type DeviceRef = { kind: "console" } | { kind: "paired"; id?: string };

/** A principal on a device. `device_name` is the paired device's name as the
 *  host's registry has it now, which serve adds on every read; absent for a
 *  device the registry no longer has. */
export type By = { principal: string; device: DeviceRef; device_name?: string };

/** Who launched the place's agent through serve; `cross_project: "off"` when
 *  a device whose scope is a list launched it (its lanes reach no other
 *  project). */
export type LaunchedBy = By & { cross_project?: string };

/** The last instruction-class file a paired device wrote into the place:
 *  `path` is relative to the place, `at` in epoch seconds. */
export type RemoteInstruction = By & { path: string; at: number };

/** The registry's names, by device id: `devices_list`, as remoteStore.ts
 *  last read it. Tombstones included, so a revoked device that edited a file
 *  is still named. */
const names = new Map<string, string>();

export function setDeviceNames(next: Iterable<[string, string]>): void {
  names.clear();
  for (const [id, name] of next) names.set(id, name);
}

/** What a device is called on screen: the name the stamp carries, else its
 *  name in the Devices list, else its id until the list has been read (or on
 *  a window that may not read it), else — a stamp that names neither — "another
 *  paired device". */
export function deviceLabel(by: By): string {
  const d = by.device;
  if (d.kind === "console") return "this computer";
  if (by.device_name) return by.device_name;
  return d.id === undefined ? "another paired device" : names.get(d.id) ?? d.id;
}

/** True for a paired device — what the place header speaks about. The
 *  console is the person at this desk; saying so would be noise. */
export function fromDevice(by?: By | null): by is By & { device: { kind: "paired"; id?: string } } {
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
  return `${name} edited from ${deviceLabel(i)}, ${agoWords(i.at, now)}`;
}

/** The longer form, for a tooltip. */
export function instructionTitle(i: RemoteInstruction): string {
  const who = i.principal === "owner" ? "" : ` (${i.principal})`;
  return `${i.path} was last written from the paired device ${deviceLabel(i)}${who} at ${new Date(i.at * 1000).toLocaleString()}. Review it in Changes before an agent reads it.`;
}

/** "from <device>" beside a live session the device launched, and why its
 *  reach may be narrower than this computer's. */
export function launchedTitle(l: LaunchedBy): string {
  const who = l.principal === "owner" ? "" : ` (${l.principal})`;
  const reach = l.cross_project === "off" ? " Its scope is a list of projects, so this agent reaches no other project." : "";
  return `This agent was launched from the paired device ${deviceLabel(l)}${who}.${reach}`;
}

// What the host app knows about remote access and its paired devices, kept
// in one place for every surface that shows it: Settings → Remote access and
// → Devices, the pairing sheet, the notice that remote access is on, the
// alerts (§5.3), and `deviceLabel` (provenance.ts), which names a device in
// the place header.
//
// Serve tells the console what changed — `remote:changed` (remote access's
// state), `devices:changed` (every registry change and pairing step) and
// `audit:alert` (§5.3's alerts, the audit log's) — and each change event here
// is a cue to re-read the row it concerns, never a copy of the state: the rows
// stay the one source. Alerts come from `audit:alert` alone, so a notice is
// exactly an alert record in the log. All three events are the console's alone
// (`scope::EVENTS`), and so are the rows: a window whose engine refuses
// `remote_status` with `console-only` is not on its host, and the panels say
// so instead of offering acts serve will refuse.
//
// The watch runs while the engine is up and starts again on every reconnect
// (`watchRemote(gen)`, from EngineGate), so a lost socket costs nothing but
// the moment it is down.
import { useSyncExternalStore } from "react";
import { invoke, listen, notGranted } from "./ipc";
import { setDeviceNames } from "./provenance";
import { alertOf, type Alert, type DeviceRecord, type RemoteStatus } from "./devices";

let alertSeq = 0;

/** How many notices stay up at once; older ones fold away. */
const MAX_ALERTS = 4;

export type RemoteState = {
  status: RemoteStatus | null;
  /** `remote_status` refused: `console-only` on a window that is not on its
   *  host, or whatever else serve said. */
  statusErr: string | null;
  devices: DeviceRecord[] | null;
  devicesErr: string | null;
  alerts: Alert[];
  /** Bumped on every pairing step serve announces, so an open pairing sheet
   *  re-reads `pair_status` at once rather than at its next tick. */
  pairTick: number;
};

let state: RemoteState = { status: null, statusErr: null, devices: null, devicesErr: null, alerts: [], pairTick: 0 };
const subs = new Set<() => void>();

function set(patch: Partial<RemoteState>) {
  state = { ...state, ...patch };
  for (const f of subs) f();
}

export function remoteState(): RemoteState {
  return state;
}

export function onRemoteState(f: () => void): () => void {
  subs.add(f);
  return () => { subs.delete(f); };
}

export function useRemote(): RemoteState {
  return useSyncExternalStore(onRemoteState, remoteState);
}

/** The device list alone: what a component that only NAMES devices
 *  (`deviceLabel`) re-renders on. */
export function useDeviceList(): DeviceRecord[] | null {
  return useSyncExternalStore(onRemoteState, () => state.devices);
}

/** The window is not on its engine's host: serve refuses the console rows. */
export function notConsole(s: RemoteState): boolean {
  return s.statusErr === "console-only";
}

/** A device's name by id, from the last `devices_list`; the id otherwise. */
export function labelOf(id: string): string {
  return state.devices?.find((d) => d.id === id)?.name ?? id;
}

// ── reads ────────────────────────────────────────────────────────────────────

let statusSeq = 0;
export function readStatus(): Promise<void> {
  const n = ++statusSeq;
  return invoke<RemoteStatus>("remote_status").then(
    (s) => {
      if (n !== statusSeq) return;
      set({ status: s, statusErr: null });
      syncQuestion(s);
    },
    (e) => { if (n === statusSeq) set({ status: null, statusErr: String(e) }); },
  );
}

let devicesSeq = 0;
export function readDevices(): Promise<void> {
  const n = ++devicesSeq;
  return invoke<DeviceRecord[]>("devices_list").then(
    (list) => {
      if (n !== devicesSeq) return;
      setDeviceNames(list.map((d) => [d.id, d.name] as [string, string]));
      set({ devices: list, devicesErr: null });
    },
    (e) => { if (n === devicesSeq) set({ devicesErr: String(e) }); },
  );
}

// ── alerts ───────────────────────────────────────────────────────────────────

/** Questions the person dismissed, by key: a question stands for as long as
 *  the pause does, and every status read finds it again, so without this a
 *  dismissed one would be back on the next `remote:changed`. */
const dismissedQ = new Set<string>();

function push(a: Alert) {
  if (state.alerts.some((x) => x.key === a.key) || dismissedQ.has(a.key)) return;
  set({ alerts: [...state.alerts, a].slice(-MAX_ALERTS) });
}

function forget(keep: (a: Alert) => boolean) {
  for (const a of state.alerts) if (!keep(a) && a.key.startsWith("q:")) dismissedQ.add(a.key);
  set({ alerts: state.alerts.filter(keep) });
}

export function dismissAlert(key: string): void {
  forget((a) => a.key !== key);
}

/** Dismiss every notice that points at `cat` — the person went to look. */
export function dismissCat(cat: Alert["cat"]): void {
  if (state.alerts.some((a) => a.cat === cat)) forget((a) => a.cat !== cat);
}

/** The question a pause puts to the host (§4.1) is an alert while it stands
 *  (§5.3: "paused by an address change, and waiting for the host to
 *  choose"), and stops being one the moment it is answered. */
function syncQuestion(s: RemoteStatus) {
  const q = s.question?.text ?? null;
  const key = q ? `q:${q}` : null;
  // Answered (or replaced): a later pause asking the same is news again.
  for (const k of dismissedQ) if (k !== key) dismissedQ.delete(k);
  const kept = state.alerts.filter((a) => !a.key.startsWith("q:") || a.key === key);
  if (kept.length !== state.alerts.length) set({ alerts: kept });
  if (key && q) push({ key, cat: "remote", payload: { alert: "question", text: q } });
}

// ── the watch ────────────────────────────────────────────────────────────────

let pending: number | null = null;
/** Coalesce a burst of registry events (a pairing is six) into one read. */
function devicesSoon() {
  if (pending !== null) return;
  pending = window.setTimeout(() => { pending = null; void readDevices(); }, 120);
}

/** Watch serve's console events while the engine `gen` is up. Returns the
 *  teardown. A window that drives a paired host is not that host's console
 *  (ipc.ts CONSOLE_ROWS): it reads no status and hears no console event —
 *  serve would send it none — and it lists devices, for their names, only
 *  when its grant reaches `devices_list` (admin). */
export function watchRemote(): () => void {
  if (!notGranted("devices_list")) void readDevices();
  if (notGranted("remote_status")) return () => {};
  let live = true;
  void readStatus();
  const uns = [
    listen<RemoteStatus>("remote:changed", () => { if (live) void readStatus(); }),
    listen<Record<string, unknown>>("devices:changed", (e) => {
      if (!live) return;
      const ev = String(e.payload?.event ?? "");
      if (ev.startsWith("pairing_")) set({ pairTick: state.pairTick + 1 });
      else devicesSoon();
      if (ev === "pairing_closed" || ev === "enrolled") devicesSoon();
    }),
    listen<Record<string, unknown>>("audit:alert", (e) => {
      if (!live) return;
      push(alertOf(e.payload, ++alertSeq));
    }),
  ];
  return () => {
    live = false;
    for (const u of uns) void u.then((f) => f());
  };
}

// ── Settings, from outside App ───────────────────────────────────────────────

let opener: ((cat: "remote" | "devices") => void) | null = null;

/** App hands over how it opens Settings at a category; a notice uses it. */
export function setSettingsOpener(f: ((cat: "remote" | "devices") => void) | null): void {
  opener = f;
}

export function openRemoteSettings(cat: "remote" | "devices"): void {
  opener?.(cat);
}

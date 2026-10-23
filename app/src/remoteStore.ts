// What the host app knows about remote access and its paired devices, kept
// in one place for every surface that shows it: Settings → Remote access and
// → Devices, the pairing sheet, the notice that remote access is on, the
// alerts (§5.3), and `deviceLabel` (provenance.ts), which names a device in
// the place header.
//
// Serve tells the console what changed — `remote:changed` (remote access's
// state), `devices:changed` (every registry change and pairing step, with
// serve's own `alert` verdict) and `audit:alert` — and each event here is a
// cue to re-read the row it concerns, never a copy of the state: the rows
// stay the one source. All three events are the console's alone
// (`scope::EVENTS`), and so are the rows: a window whose engine refuses
// `remote_status` with `console-only` is not on its host, and the panels say
// so instead of offering acts serve will refuse.
//
// The watch runs while the engine is up and starts again on every reconnect
// (`watchRemote(gen)`, from EngineGate), so a lost socket costs nothing but
// the moment it is down.
import { useSyncExternalStore } from "react";
import { invoke, listen } from "./ipc";
import { setDeviceNames } from "./provenance";
import { alertOf, type Alert, type DeviceRecord, type RemoteStatus } from "./devices";

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

function push(a: Alert) {
  if (state.alerts.some((x) => x.key === a.key)) return;
  set({ alerts: [...state.alerts, a].slice(-MAX_ALERTS) });
}

export function dismissAlert(key: string): void {
  set({ alerts: state.alerts.filter((a) => a.key !== key) });
}

/** Dismiss every notice that points at `cat` — the person went to look. */
export function dismissCat(cat: Alert["cat"]): void {
  if (state.alerts.some((a) => a.cat === cat)) set({ alerts: state.alerts.filter((a) => a.cat !== cat) });
}

/** The question a pause puts to the host (§4.1) is an alert while it stands
 *  (§5.3: "paused by an address change, and waiting for the host to
 *  choose"), and stops being one the moment it is answered. */
function syncQuestion(s: RemoteStatus) {
  const q = s.question?.text ?? null;
  const key = q ? `q:${q}` : null;
  const kept = state.alerts.filter((a) => !a.key.startsWith("q:") || a.key === key);
  if (kept.length !== state.alerts.length) set({ alerts: kept });
  if (key && q) push({ key, cat: "remote", text: q });
}

// ── the watch ────────────────────────────────────────────────────────────────

let pending: number | null = null;
/** Coalesce a burst of registry events (a pairing is six) into one read. */
function devicesSoon() {
  if (pending !== null) return;
  pending = window.setTimeout(() => { pending = null; void readDevices(); }, 120);
}

/** Watch serve's console events while the engine `gen` is up. Returns the
 *  teardown. */
export function watchRemote(): () => void {
  let live = true;
  void readStatus();
  void readDevices();
  const uns = [
    listen<RemoteStatus>("remote:changed", () => { if (live) void readStatus(); }),
    listen<Record<string, unknown>>("devices:changed", (e) => {
      if (!live) return;
      const ev = String(e.payload?.event ?? "");
      if (ev.startsWith("pairing_")) set({ pairTick: state.pairTick + 1 });
      else devicesSoon();
      if (ev === "pairing_closed" || ev === "enrolled") devicesSoon();
      const a = alertOf("devices:changed", e.payload, labelOf, Date.now());
      if (a) push(a);
    }),
    listen<Record<string, unknown>>("audit:alert", (e) => {
      if (!live) return;
      const a = alertOf("audit:alert", e.payload, labelOf, Date.now());
      if (a) push(a);
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

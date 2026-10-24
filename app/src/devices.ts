// Remote access and paired devices, as the host app shows them (Server
// Edition Phase 2, `docs/proposals/server-edition-phase2.md` §1.1, §2.2–§2.5,
// §3.1, §4.1, §5.3). The rows are serve's console rows (`remote_*`,
// `pair_*`, `device_*`, `devices_list`); this file is the words and the
// shapes, and nothing else. Pure on purpose — no React, no invoke, no import:
// `app/scripts/devices-check.mjs` loads it as a `data:` module and holds its
// mirrors (the presets, the expiry bounds) against the Rust they copy.

export type Cap = "read" | "write" | "answer" | "instruct" | "stream" | "admin";
/** `authz::Capability`, in its own order. */
export const CAPS: readonly Cap[] = ["read", "write", "answer", "instruct", "stream", "admin"];
export type Projects = "*" | string[];
export type Expiry = { idle_days: number } | { until: string } | "never";
export type Platform = "ios" | "ipados" | "macos" | "linux";

/** `devices::DeviceRecord`, as `devices_list` answers it. */
export type DeviceRecord = {
  id: string;
  principal: string;
  name: string;
  platform: Platform;
  spki_sha256: string;
  stepup_spki_sha256: string | null;
  stepup_required: boolean;
  caps: Cap[];
  projects: Projects;
  expiry: Expiry;
  enrolled_at: number;
  last_seen: number | null;
  last_addr: string | null;
  disabled: { at: number; reason: "idle" | "until" | "unrecorded" | "console" } | null;
  revoked_at: number | null;
  reenabled_at?: number | null;
};

/** `remote_status`. */
export type RemoteStatus = {
  state: "off" | "unreadable" | "listening" | "paused" | "error" | "starting";
  owner?: string | null;
  host?: string;
  error?: string | null;
  port?: number | null;
  advertise?: boolean;
  address?: string;
  iface?: string;
  mode?: "lan" | "overlay";
  at?: string | null;
  pin?: string | null;
  address_code?: string | null;
  paused?: string | null;
  question?: { text: string; alert: boolean; use_address: string } | null;
  warning?: string | null;
  advert?: { state: "registered"; host: string } | { state: "not-advertised"; why: string | null } | { state: "off" };
};

/** One row of `remote_candidates`. */
export type Candidate = {
  address: string;
  prefix: number;
  iface: string;
  class: "rfc1918" | "cgnat" | "ula" | "link-local";
  mode: "lan" | "overlay";
  warning: string | null;
};

/** `pairing::Ended`. */
export type Ended =
  | "paired" | "denied" | "expired" | "undecided" | "cancelled" | "superseded" | "refused"
  | "bad_tls" | "malformed" | "bad_proof" | "second_connection" | "frame_after_first" | "device_left";

export type PairRequest = {
  name: string;
  platform: Platform;
  code: string;
  peer: string;
  grant: { capabilities: Cap[]; projects: Projects };
  expiry: Expiry;
};

/** `pair_status` (`pairing::Status`). */
export type PairStatus =
  | { state: "none" }
  | { state: "open"; uri: string; expires_at: number }
  | { state: "attempt"; peer: string; expires_at: number }
  | { state: "pending"; request: PairRequest; decide_by: number }
  | { state: "closed"; ended: Ended; device: string | null };

/** `pair_begin`'s answer. `qr.modules` is row-major, `1` dark. */
export type PairBegin = { uri: string; expires_at: number; pin: string; host: string; qr: { size: number; modules: string } };

// ── capabilities ─────────────────────────────────────────────────────────────

/** §3.1's presets; `admin` is a separate toggle on top of any of them. The
 *  same sets as `worktrees serve pair --preset` (client.rs `preset`). */
export const PRESETS = {
  observe: ["read"],
  operate: ["read", "write"],
  drive: ["read", "write", "answer", "instruct", "stream"],
} as const satisfies Record<string, readonly Cap[]>;
export type Preset = keyof typeof PRESETS;
export const PRESET_LABEL: Record<Preset | "custom", string> = { observe: "Observe", operate: "Operate", drive: "Drive", custom: "Custom" };

/** The capabilities in `CAPS` order, each once. */
export function normCaps(caps: readonly Cap[]): Cap[] {
  return CAPS.filter((c) => caps.includes(c));
}

/** Which preset `caps` is, `admin` aside; `custom` for any other set. */
export function presetOf(caps: readonly Cap[]): Preset | "custom" {
  const base = normCaps(caps).filter((c) => c !== "admin").join(",");
  for (const p of Object.keys(PRESETS) as Preset[]) if (PRESETS[p].join(",") === base) return p;
  return "custom";
}

/** §1.1's label, computed and never chosen: the device can run commands on
 *  the host as you when it holds `stream`, or `answer` with `write` or
 *  `instruct`, or `instruct` while host policy lets agents act unprompted
 *  (Codex permissions other than `ask`). */
export function runsCommands(caps: readonly Cap[], unprompted: boolean): boolean {
  const has = (c: Cap) => caps.includes(c);
  return has("stream") || (has("answer") && (has("write") || has("instruct"))) || (has("instruct") && unprompted);
}

export function runsCommandsLabel(host: string | undefined): string {
  return `can run commands on ${host || "this computer"} as you`;
}

/** What one capability lets a device do, for the custom picker. */
export const CAP_WORDS: Record<Cap, string> = {
  read: "read state, diffs and files",
  write: "edit files, create and close places, launch agents",
  answer: "answer agents' prompts",
  instruct: "write what agents obey (briefs, AGENTS.md)",
  stream: "terminals and shells",
  admin: "host settings that widen no agent",
};

/** "Observe", "Drive + host admin", or the list for a custom set. */
export function capsWords(caps: readonly Cap[]): string {
  const p = presetOf(caps);
  const admin = caps.includes("admin") ? " + host admin" : "";
  if (p !== "custom") return PRESET_LABEL[p] + admin;
  const rest = normCaps(caps).filter((c) => c !== "admin");
  return (rest.length ? rest.join(", ") : "nothing") + admin;
}

/** "every project", "1 project", "3 projects". */
export function scopeWords(p: Projects): string {
  if (p === "*") return "every project";
  return p.length === 1 ? "1 project" : `${p.length} projects`;
}

// ── expiry ───────────────────────────────────────────────────────────────────

/** devices.rs `DEFAULT_IDLE_DAYS` and `MAX_IDLE_DAYS`. */
export const DEFAULT_IDLE_DAYS = 30;
export const MAX_IDLE_DAYS = 3650;
export const DEFAULT_EXPIRY: Expiry = { idle_days: DEFAULT_IDLE_DAYS };

/** `devices::rfc3339`'s one spelling: `YYYY-MM-DDTHH:MM:SSZ`, UTC. */
export function rfc3339(ms: number): string {
  return new Date(Math.floor(ms / 1000) * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");
}

/** A local calendar day (`YYYY-MM-DD`, a date input's value) as the moment it
 *  ENDS, in the spelling serve takes: a device "until 12 Jan" works all day. */
export function untilEndOf(day: string): string | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return null;
  const end = new Date(+m[1], +m[2] - 1, +m[3], 23, 59, 59);
  return Number.isNaN(end.getTime()) ? null : rfc3339(end.getTime());
}

/** The local day an `until` falls on, for a date input. */
export function dayOf(until: string): string {
  const d = new Date(until);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** "disabled after 30 days unused", "until 12 Jan 2027", "never expires". */
export function expiryWords(e: Expiry): string {
  if (e === "never") return "never expires";
  if ("idle_days" in e) return `disabled after ${e.idle_days} ${e.idle_days === 1 ? "day" : "days"} unused`;
  return `until ${new Date(e.until).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })}`;
}

// ── the record, in words ─────────────────────────────────────────────────────

export const PLATFORM_LABEL: Record<Platform, string> = { ios: "iPhone", ipados: "iPad", macos: "Mac", linux: "Linux" };

/** The platform enrolls a presence-bound step-up key (devices.rs
 *  `has_presence_key`). */
export function hasPresenceKey(p: Platform): boolean {
  return p === "ios" || p === "ipados";
}

const DISABLED_WORDS: Record<NonNullable<DeviceRecord["disabled"]>["reason"], string> = {
  idle: "its idle expiry ran out",
  until: "its expiry date passed",
  unrecorded: "its record said more than serve had written",
  console: "paused on this computer",
};

/** `live`, `disabled` (and why) or `revoked`. */
export function deviceState(d: DeviceRecord): { kind: "live" | "disabled" | "revoked"; words: string } {
  if (d.revoked_at != null) return { kind: "revoked", words: "Revoked" };
  if (d.disabled) return { kind: "disabled", words: `Disabled — ${DISABLED_WORDS[d.disabled.reason]}` };
  return { kind: "live", words: "Allowed" };
}

/** "just now", "5 min ago", "3 d ago". */
export function ago(epoch: number, nowMs: number = Date.now()): string {
  const s = Math.max(0, Math.floor(nowMs / 1000 - epoch));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

/** "Last seen 5 min ago from 192.168.1.7", or never. */
export function seenWords(d: DeviceRecord, nowMs?: number): string {
  if (d.last_seen == null) return d.last_addr ? `Not connected yet · paired from ${d.last_addr}` : "Not connected yet";
  return `Last seen ${ago(d.last_seen, nowMs)}${d.last_addr ? ` from ${d.last_addr}` : ""}`;
}

/** What the device list says about step-up (§2.5): on, off (a factor
 *  removed, said for as long as it stays off), or a desktop's one factor for
 *  drive-class acts. `null` when there is nothing to say. */
export function stepupWords(d: DeviceRecord, unprompted: boolean): string | null {
  if (hasPresenceKey(d.platform)) {
    return d.stepup_required ? "Step-up on: drive-class acts ask for Face ID or the passcode" : "Step-up OFF: drive-class acts need no Face ID or passcode";
  }
  return runsCommands(d.caps, unprompted) || d.caps.includes("admin") ? "No presence key: drive-class acts rest on the device key alone" : null;
}

// ── pairing ──────────────────────────────────────────────────────────────────

/** "482 913": the comparison code, in two groups of three. */
export function codeSpaced(code: string): string {
  return /^\d{6}$/.test(code) ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

/** Whole seconds until `epoch`, never below zero. */
export function secondsLeft(epoch: number, nowMs: number = Date.now()): number {
  return Math.max(0, Math.ceil(epoch - nowMs / 1000));
}

/** How a window ended, in the host's words. */
export const ENDED_WORDS: Record<Ended, string> = {
  paired: "Paired.",
  denied: "Denied. The window is closed.",
  expired: "No device tried in time. The window is closed.",
  undecided: "No answer within 60 seconds, so the request was denied.",
  cancelled: "Cancelled. The window is closed.",
  superseded: "Another pairing window replaced this one.",
  refused: "Allowed, but the device could not be enrolled.",
  bad_tls: "The device's connection failed. The window is closed.",
  malformed: "The device sent a request serve could not read. The window is closed.",
  bad_proof: "The device's proof did not match this window. The window is closed; open a new one to try again.",
  second_connection: "A second connection reached the window, so it closed. If that was not your device, someone else saw the code.",
  frame_after_first: "The device sent more than one request, so the window closed.",
  device_left: "The device left before you answered. The window is closed.",
};

/** The QR's dark modules as one SVG path in module units: a run of dark
 *  modules in a row is one rectangle. */
export function qrPath(size: number, modules: string): string {
  if (size <= 0 || modules.length !== size * size) return "";
  let d = "";
  for (let y = 0; y < size; y++) {
    let x = 0;
    while (x < size) {
      if (modules[y * size + x] !== "1") { x++; continue; }
      const from = x;
      while (x < size && modules[y * size + x] === "1") x++;
      d += `M${from} ${y}h${x - from}v1h${from - x}z`;
    }
  }
  return d;
}

// ── alerts (§5.3) ────────────────────────────────────────────────────────────

/** One notice for the person at the host: `audit:alert`'s payload, kept as
 *  data and worded when it is drawn (a device paired a moment ago is named
 *  once the list has been re-read). `key` dedupes and dismisses. */
export type Alert = { key: string; cat: "devices" | "remote"; payload: Record<string, unknown> };

type Subject = { principal: string; device: string };

/** `devices::ConnectRefusal`, as the audit log spells a knock's reason
 *  (`unknown-key` is never an alert). */
const REFUSED_WORDS: Record<string, string> = {
  revoked: "it is revoked",
  disabled: "it is disabled",
  expired: "its expiry ran out, so it is disabled now",
  unreadable: "devices.json cannot be read",
};

/** The notice for one `audit:alert`. The audit log is §5.3's one alert
 *  channel: serve decides what is an alert (`DeviceEvent::alerts`, and its
 *  episodes, so a refused device that keeps knocking alerts once per
 *  episode), and every one it emits is a notice — the page adds none and
 *  drops none. `at` keys it. */
export function alertOf(payload: unknown, at: number): Alert {
  const p = (payload && typeof payload === "object" ? payload : {}) as Record<string, unknown>;
  return { key: `audit:${at}:${String(p.alert)}:${String(p.event ?? "")}`, cat: p.alert === "device" ? "devices" : "remote", payload: p };
}

/** An alert in words. `label` names a device id. */
export function alertText(a: Alert, label: (id: string) => string): string {
  const p = a.payload;
  if (p.alert === "auth_failures") {
    const since = p.since ? ` since ${new Date(String(p.since)).toLocaleTimeString()}` : "";
    return `${p.count ?? "Several"} connection attempts failed authentication${since}.`;
  }
  if (p.alert !== "device") return `Security alert: ${String(p.alert ?? "see the audit log")}.`;
  const who = (p.subject as Subject | null)?.device ? label((p.subject as Subject).device) : "A device";
  switch (p.event) {
    case "enrolled":
      return `${who} was paired with this computer.`;
    case "new_address_class":
      return `${who} connected from a different kind of network than before.`;
    case "disabled":
      return `${who} was disabled: ${DISABLED_WORDS[p.reason as keyof typeof DISABLED_WORDS] ?? String(p.reason)}.`;
    case "connect_refused":
      return `${who} tried to connect and was refused: ${REFUSED_WORDS[String(p.reason)] ?? String(p.reason)}.`;
    case "unreadable":
      return "devices.json cannot be read, so every device is refused until it is fixed.";
    case "identity_rotated":
      return "This computer's server identity was rotated: every device must pair again.";
    default:
      return `Device alert: ${String(p.event)}.`;
  }
}

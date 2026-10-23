// Settings → Devices (Phase 2 §2.5, §3.2, §5.3): every device paired with
// this computer, what each may do, until when, when it was last seen and
// from where, and the host's acts on it — rename, disable or re-enable,
// expiry, scope, step-up, revoke. Each act is a console row; a refusal is
// shown on the row it came from, in serve's words. The list is
// `devices_list`, re-read on every `devices:changed` (remoteStore.ts).
import { useState } from "react";
import { invoke } from "./ipc";
import { dismissCat, notConsole, readDevices, useRemote } from "./remoteStore";
import { ConfirmDialog, ExpiryEditor, RunsLabel, ScopeEditor, scopeProblem, useNow, useProjects, useUnprompted, type ProjectRef, type Scope } from "./remoteui";
import { PairSheet } from "./PairSheet";
import {
  capsWords, deviceState, expiryWords, hasPresenceKey, PLATFORM_LABEL, scopeWords, seenWords, stepupWords,
  type DeviceRecord, type Expiry,
} from "./devices";
import "./RemoteAccess.css";

/** devices.rs `MAX_DEVICES`: revoked tombstones do not count. */
const MAX_DEVICES = 16;

export function DevicesSection() {
  const r = useRemote();
  const [pairing, setPairing] = useState(false);
  const [showRevoked, setShowRevoked] = useState(false);
  const unprompted = useUnprompted();
  const projects = useProjects();
  const now = useNow(30_000);

  if (notConsole(r)) {
    return (
      <section className="setting ra" data-focus="devices">
        <label>Paired devices</label>
        <div className="ra-gate">Devices are paired and managed on the computer that runs this engine, not from a paired device.</div>
      </section>
    );
  }
  const list = r.devices ?? [];
  const current = list.filter((d) => d.revoked_at == null);
  const revoked = list.filter((d) => d.revoked_at != null);
  const listening = r.status?.state === "listening";
  const full = current.length >= MAX_DEVICES;
  const why = !listening
    ? "A device pairs over remote access: turn it on in Settings → Remote access first."
    : full ? `${MAX_DEVICES} devices are paired: revoke one before pairing another.` : null;
  return (
    <>
      <section className="setting ra" data-focus="devices">
        <label>Paired devices</label>
        <div className="ra-line">
          {current.length === 0 ? "No device is paired with this computer." : `${current.length} of ${MAX_DEVICES} devices.`} A device
          reaches only what it was granted, from a key that never leaves it, and only while remote access is on.
        </div>
        <div className="ver-actions">
          <button className="enter-btn sm" disabled={!!why} title={why ?? undefined} onClick={() => { dismissCat("devices"); setPairing(true); }}>
            Pair a device…
          </button>
        </div>
        {why && <div className="ra-note" data-testid="pair-why">{why}</div>}
        {r.devicesErr && <div className="ra-err" role="alert">The device list could not be read: {r.devicesErr}</div>}
      </section>
      {current.length > 0 && (
        <section className="setting ra ra-devices">
          {current.map((d) => <DeviceRow key={d.id} d={d} unprompted={unprompted} host={r.status?.host} projects={projects} now={now} />)}
        </section>
      )}
      {revoked.length > 0 && (
        <section className="setting ra">
          <button className="ra-linkbtn" aria-expanded={showRevoked} onClick={() => setShowRevoked((v) => !v)}>
            {showRevoked ? "Hide" : "Show"} revoked devices ({revoked.length})
          </button>
          {showRevoked && (
            <>
              <div className="ra-note">Kept as tombstones for the audit trail. A revoked device never connects again; pairing it again makes a new one.</div>
              {revoked.map((d) => (
                <div key={d.id} className="ra-dev revoked" data-device={d.id}>
                  <div className="ra-dev-head">
                    <span className="ra-dot off" aria-hidden />
                    <b className="ra-dev-name">{d.name}</b>
                    <span className="ra-dev-meta">{PLATFORM_LABEL[d.platform]} · revoked {new Date((d.revoked_at ?? 0) * 1000).toLocaleDateString()}</span>
                  </div>
                </div>
              ))}
            </>
          )}
        </section>
      )}
      {pairing && <PairSheet onClose={() => { setPairing(false); void readDevices(); }} />}
    </>
  );
}

type Editing = null | "rename" | "expiry" | "scope" | "revoke";

function DeviceRow({ d, unprompted, host, projects, now }: {
  d: DeviceRecord;
  unprompted: boolean;
  host?: string;
  projects: ProjectRef[] | null;
  now: number;
}) {
  const [editing, setEditing] = useState<Editing>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [name, setName] = useState(d.name);
  const [expiry, setExpiry] = useState<Expiry>(d.expiry);
  const [scope, setScope] = useState<Scope>({ caps: d.caps, projects: d.projects });
  const s = deviceState(d);
  const step = stepupWords(d, unprompted);
  const projName = (root: string) => projects?.find((p) => p.root === root)?.name ?? root.split("/").pop() ?? root;

  /** One console act on this device; its refusal stays on this row. */
  const act = async (cmd: string, args: Record<string, unknown>, after?: () => void) => {
    setBusy(true);
    setErr(null);
    try {
      await invoke(cmd, { id: d.id, ...args });
      setEditing(null);
      after?.();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
      void readDevices();
    }
  };
  const open = (what: Editing) => {
    setErr(null);
    setName(d.name);
    setExpiry(d.expiry);
    setScope({ caps: d.caps, projects: d.projects });
    setEditing((cur) => (cur === what ? null : what));
  };

  return (
    <div className={`ra-dev ${s.kind}`} data-device={d.id}>
      <div className="ra-dev-head">
        <span className={`ra-dot ${s.kind === "live" ? "on" : "warn"}`} aria-hidden />
        <b className="ra-dev-name">{d.name}</b>
        <span className="ra-dev-meta">{PLATFORM_LABEL[d.platform]}</span>
        <span className="ra-dev-state">{s.words}</span>
      </div>
      <div className="ra-dev-facts">
        <div>
          {capsWords(d.caps)} on {d.projects === "*" ? "every project" : <span title={d.projects.join("\n")}>{scopeWords(d.projects)}: {d.projects.map(projName).join(", ")}</span>}
        </div>
        <RunsLabel caps={d.caps} unprompted={unprompted} host={host} />
        <div>{d.expiry === "never" ? <b>Never expires</b> : expiryWords(d.expiry)}</div>
        <div>{seenWords(d, now)}</div>
        {step && <div className={d.stepup_required ? undefined : "ra-dev-flag"}>{step}</div>}
      </div>
      {err && <div className="ra-err" role="alert">{err}</div>}
      <div className="ver-actions ra-dev-actions">
        <button className="ctrl sm" disabled={busy} onClick={() => open("rename")}>Rename</button>
        {s.kind === "live"
          ? <button className="ctrl sm" disabled={busy} onClick={() => act("device_disable", {})}>Disable</button>
          : <button className="ctrl sm" disabled={busy} onClick={() => act("device_enable", {})}>Enable</button>}
        <button className="ctrl sm" disabled={busy} onClick={() => open("expiry")}>Expiry…</button>
        <button className="ctrl sm" disabled={busy} onClick={() => open("scope")}>Scope…</button>
        {hasPresenceKey(d.platform) && (
          <button className="ctrl sm" disabled={busy} onClick={() => act("device_set_stepup", { required: !d.stepup_required })}>
            {d.stepup_required ? "Turn step-up off" : "Turn step-up on"}
          </button>
        )}
        <button className="ctrl sm ra-danger" disabled={busy} onClick={() => open("revoke")}>Revoke…</button>
      </div>
      {editing === "rename" && (
        <form className="ra-edit" onSubmit={(e) => { e.preventDefault(); void act("device_rename", { name: name.trim() }); }}>
          <input className="ra-name-input" value={name} maxLength={64} autoFocus aria-label="Device name" onChange={(e) => setName(e.currentTarget.value)} />
          <button className="ctrl sm" type="submit" disabled={busy || !name.trim()}>Save</button>
          <button className="ctrl sm" type="button" onClick={() => setEditing(null)}>Cancel</button>
        </form>
      )}
      {editing === "expiry" && (
        <div className="ra-edit col">
          <ExpiryEditor value={expiry} onChange={setExpiry} />
          {s.kind === "disabled" && d.disabled?.reason === "until" && <div className="ra-note">Set a later date, then Enable.</div>}
          <div className="ver-actions">
            <button className="ctrl sm" disabled={busy} onClick={() => act("device_set_expiry", { expiry })}>Save expiry</button>
            <button className="ctrl sm" onClick={() => setEditing(null)}>Cancel</button>
          </div>
        </div>
      )}
      {editing === "scope" && (
        <div className="ra-edit col">
          <ScopeEditor value={scope} onChange={setScope} projects={projects} unprompted={unprompted} host={host} />
          <div className="ra-note">Taking something away ends the device's open terminals that it no longer covers, at once.</div>
          <div className="ver-actions">
            <button className="ctrl sm" disabled={busy || !!scopeProblem(scope)} title={scopeProblem(scope) ?? undefined}
              onClick={() => act("device_set_scope", { caps: scope.caps, projects: scope.projects })}>Save scope</button>
            <button className="ctrl sm" onClick={() => setEditing(null)}>Cancel</button>
          </div>
        </div>
      )}
      {editing === "revoke" && (
        <ConfirmDialog title={`Revoke ${d.name}?`} confirm="Revoke" danger busy={busy} error={err}
          onConfirm={() => act("device_revoke", {})} onClose={() => setEditing(null)}>
          <div className="ra-line">
            It is disconnected now and can never connect again: pairing it again is the only way back. Its record stays as a
            tombstone for the audit trail.
          </div>
        </ConfirmDialog>
      )}
    </div>
  );
}

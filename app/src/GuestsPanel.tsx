// Settings → Remote → Guests (group chat §1, §2.1, §2.2): the people who are
// not the owner. *Invite a guest* opens the pairing sheet as an invite (a
// name, no scope: a guest holds `guest` and nothing else); each guest is
// listed with their devices and a Remove that takes them off every share;
// the owner's own name, as guests and the thread see it; and, per project,
// *Show place names to guests* (default on, host state, never the repo's).
// Every row is the console's.
import { useState } from "react";
import { invoke } from "./ipc";
import { PairSheet } from "./PairSheet";
import { ConfirmDialog, useProjects } from "./remoteui";
import { readShares, useShares, guestLive, type Guest } from "./shares";
import "./RemoteAccess.css";

function when(at: number | null | undefined): string {
  if (!at) return "never";
  return new Date(at * 1000).toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" });
}

export function GuestsSection({ onRemote }: { onRemote: () => void }) {
  const s = useShares();
  const projects = useProjects();
  const [inviting, setInviting] = useState<{ id: string | null; name: string } | null>(null);
  const [removing, setRemoving] = useState<Guest | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [name, setName] = useState<string | null>(null);

  const act = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setErr(null);
    try {
      await f();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
      void readShares();
    }
  };
  const shownName = name ?? (s.owner === "owner" ? "" : s.owner);
  const sharesOfGuest = (g: Guest) => (s.shares ?? []).filter((x) => x.guests.some((y) => y.principal === g.id)).length;

  return (
    <section className="setting ra" data-testid="guests">
      <label>Guests</label>
      <div className="ra-note">
        A guest is another person you pair with this computer. They see only the sessions you share with them, once you let them in, and
        the names of your places in projects you have not hidden. Nothing they write reaches an agent unless you send it.
      </div>

      <div className="ra-kv" data-testid="owner-name">
        <span>Your name, as guests see it</span>
        <input value={shownName} placeholder="David" maxLength={64} aria-label="Your name, as guests see it" onChange={(e) => setName(e.target.value)} />
        <button className="ctrl sm" disabled={busy || !shownName.trim() || shownName.trim() === s.owner}
          onClick={() => act(() => invoke("owner_set_name", { name: shownName.trim() }).then(() => setName(null)))}>
          Save
        </button>
      </div>

      <div className="ver-actions">
        <button className="enter-btn sm" data-testid="invite-guest" onClick={() => setInviting({ id: null, name: "" })}>Invite a guest</button>
      </div>

      {s.guests.length === 0 ? (
        <div className="ra-note">No guests yet.</div>
      ) : (
        <ul className="ra-guests" aria-label="Guests">
          {s.guests.map((g) => (
            <li key={g.id} className="ra-guest" data-guest={g.id}>
              <div className="ra-guest-head">
                <span className={`ra-dot ${guestLive(g) ? "on" : "off"}`} aria-hidden />
                <b>{g.name}</b>
                <span className="ra-note">since {when(g.created_at)} · {sharesOfGuest(g)} {sharesOfGuest(g) === 1 ? "share" : "shares"}</span>
                <span className="ra-guest-acts">
                  <button className="ctrl sm" onClick={() => setInviting({ id: g.id, name: g.name })}>Pair another device</button>
                  <button className="ctrl sm danger" disabled={busy} onClick={() => setRemoving(g)}>Remove</button>
                </span>
              </div>
              <ul className="ra-guest-devs">
                {g.devices.length === 0 && <li className="ra-note">No device.</li>}
                {g.devices.map((d) => (
                  <li key={d.id} className="ra-note">
                    {d.name}
                    {d.revoked_at ? ` · removed ${when(d.revoked_at)}` : ` · last seen ${when(d.last_seen as number | null)}`}
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      )}

      <div className="ra-field-l" style={{ marginTop: "var(--s3)" }}>Show place names to guests</div>
      <div className="ra-note">
        A guest sees the names of your places (a slug is usually a branch; a title is what you or an agent wrote) in every project switched on
        here, and nothing inside them. A hidden project disappears for guests, its name included; you can still share a place in it.
      </div>
      {!projects && <div className="ra-note">Reading the projects…</div>}
      {projects?.map((p) => {
        const hidden = s.hidden.includes(p.root);
        return (
          <label key={p.root} className="tier-toggle setting-check" data-project={p.root}>
            <input type="checkbox" checked={!hidden} disabled={busy}
              onChange={(e) => { const on = e.currentTarget.checked; void act(() => invoke("guest_places_hide", { project: p.root, hidden: !on })); }} />
            {p.name}
          </label>
        );
      })}

      {(err || s.error) && <div className="ra-err" role="alert">{err ?? s.error}</div>}

      {inviting && (
        <PairSheet guest={inviting} onRemote={onRemote} onClose={() => { setInviting(null); void readShares(); }} />
      )}
      {removing && (
        <ConfirmDialog title={`Remove ${removing.name}?`} confirm="Remove" danger busy={busy} error={err}
          onConfirm={() => act(() => invoke("guest_remove", { principal: removing.id }).then(() => setRemoving(null)))}
          onClose={() => setRemoving(null)}>
          <div className="ra-line">
            Every device of {removing.name}'s is revoked and its connection closes now. They are taken off every share; the threads keep what
            they wrote. What they have already read, they have read: this can't be recalled.
          </div>
        </ConfirmDialog>
      )}
    </section>
  );
}

// The sidebar's guest rows (group chat §7, "Bret's sidebar"): what a host this
// machine is a guest of has shared, and its owner's other place names.
//
// Placement is COSMETIC (§7): a share and a project's place names go under the
// local project with the same NAME, as *David's Places*, and everything else
// under one *Shared with me* section at the end. A wrong match puts a row in
// the wrong place and does nothing else: nothing acts on a row but opening it
// (a share) or asking for it (a name). The host sends the project's folder
// name, not its origin URL, so a name is what is compared.
//
// A shared row says where its guest stands: invited (opening asks to join),
// waiting for the owner, live, or ended (frozen). An unshared name has one
// act, *Ask <owner> to share*, with an optional note; the host answers the
// same for every name (§2.2), so the row only says the ask went.
import { useState } from "react";
import { dismissToast, forgetGuestHost, openShare, requestShare, retryGuestHost, useGuests, type GuestHostView, type GuestProject, type GuestShare } from "./guest";
import { placed, removedWords, shareWords } from "./guestview";
import { PLAIN_TEXT } from "./thread";
import "./GuestSession.css";

/** `shares::NOTE_MAX`: a request's note, in bytes. */
export const NOTE_MAX = 280;
const bytes = (s: string) => new TextEncoder().encode(s).length;

function ShareRow({ hv, s, selected }: { hv: GuestHostView; s: GuestShare; selected: boolean }) {
  const w = shareWords(s, hv.owner);
  const unseen = hv.unseen.includes(s.id);
  return (
    <li
      className={"row guest-row" + (selected ? " sel" : "")}
      data-testid="guest-share-row"
      data-share={s.id}
      title={`${hv.owner} shared ${s.title || s.slug} (${s.project}) with you`}
      onClick={() => openShare(hv.host.id, s.id)}
    >
      <span className={"status-dot guest-dot " + w.cls} />
      <span className="row-id">
        <span className="row-name">{s.title || s.slug}</span>
        <span className="row-branch guest-state">{w.words}</span>
      </span>
      {unseen && <span className="guest-new" aria-label="new">new</span>}
    </li>
  );
}

function NameRow({ hv, project, slug, title }: { hv: GuestHostView; project: GuestProject; slug: string; title: string | null }) {
  const [asking, setAsking] = useState(false);
  const [note, setNote] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const asked = hv.asked.includes(`${project.id}|${slug}`);
  const over = bytes(note) > NOTE_MAX;
  const send = async () => {
    setBusy(true);
    setErr(null);
    try {
      await requestShare(hv.host.id, project.id, slug, note.trim() || null);
      setAsking(false);
      setNote("");
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <li className="row guest-name" data-testid="guest-name-row" title={title ? `${title} — ${slug}` : slug}>
      <span className="status-dot guest-dot none" />
      <span className="row-id">
        <span className="row-name">{title || slug}</span>
      </span>
      {asked
        ? <span className="guest-asked" data-testid="guest-asked-mark">Asked</span>
        : <button className="guest-ask-btn" data-testid="guest-ask" disabled={hv.link.kind !== "up"} title={`Ask ${hv.owner} to share this session with you`} onClick={(e) => { e.stopPropagation(); setAsking(!asking); }}>Ask</button>}
      {asking && !asked && (
        // A real form, so the button and Enter are one submit. It spans the
        // row's grid (`.guest-askform`): as the row's fourth child it fell
        // into the 14px dot column, one word per line.
        <form className="guest-askform" data-testid="guest-askform" onClick={(e) => e.stopPropagation()}
          ref={(f) => { if (f && !f.dataset.shown) { f.dataset.shown = "1"; f.scrollIntoView({ block: "nearest" }); } }}
          onSubmit={(e) => { e.preventDefault(); if (!over && !busy) void send(); }}>
          <div className="hint">Ask {hv.owner} to share {title || slug}. It grants nothing: {hv.owner} shares it, or not.</div>
          <input type="text" value={note} placeholder="A note (optional)" aria-label="A note for the ask" maxLength={NOTE_MAX * 2} autoFocus {...PLAIN_TEXT}
            onChange={(e) => setNote(e.currentTarget.value)} />
          <div className="ver-actions">
            <button type="submit" className="ctrl sm" data-testid="guest-ask-send" disabled={busy || over || hv.link.kind !== "up"}>Ask {hv.owner} to share</button>
            <button type="button" className="ctrl sm" onClick={() => setAsking(false)}>Cancel</button>
          </div>
          {hv.link.kind !== "up" && <div className="hint">{hv.owner}’s host is not connected; the ask goes once it is.</div>}
          {over && <div className="hint">The note is {bytes(note)} bytes; at most {NOTE_MAX}.</div>}
          {err && <div className="hint" role="alert">{err}</div>}
        </form>
      )}
    </li>
  );
}

/** A host that ended this device: why, a Retry (a disabled device can be
 *  turned back on there) and *Forget this host*, a two-click arm. */
function RemovedRow({ hv }: { hv: GuestHostView }) {
  const [armed, setArmed] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  if (hv.link.kind !== "removed") return null;
  const forget = async () => {
    if (!armed) { setArmed(true); window.setTimeout(() => setArmed(false), 4000); return; }
    setErr(null);
    try { await forgetGuestHost(hv.host.id); } catch (e) { setErr(String(e)); setArmed(false); }
  };
  return (
    <li className="row guest-name guest-removed" data-testid="guest-removed" title={hv.link.why}>
      <span className="status-dot guest-dot none" />
      <span className="row-id">
        <span className="row-name guest-state">{removedWords(hv.link.by, hv.owner)}.</span>
      </span>
      <div className="guest-removed-acts">
        <button className="ctrl sm" data-testid="guest-retry" onClick={(e) => { e.stopPropagation(); retryGuestHost(hv.host.id); }}>Retry</button>
        <button className={"ctrl sm danger" + (armed ? " armed" : "")} data-testid="guest-forget" onClick={(e) => { e.stopPropagation(); void forget(); }}>
          {armed ? "Really forget?" : "Forget this host"}
        </button>
      </div>
      {err && <div className="hint" role="alert">{err}</div>}
    </li>
  );
}

/** One host's group: its shares, then its other place names by project. */
function HostGroup({ hv, shares, projects, heading, openShareId }: { hv: GuestHostView; shares: GuestShare[]; projects: GuestProject[]; heading: string; openShareId: string | null }) {
  const [open, setOpen] = useState(true);
  if (!shares.length && !projects.length && hv.link.kind === "up") return null;
  const unseen = shares.filter((s) => hv.unseen.includes(s.id)).length;
  const sharedSlugs = new Set(shares.map((s) => `${s.project}|${s.slug}`));
  return (
    <div className="group guest-group" data-testid="guest-group">
      <div className="group-h" onClick={() => setOpen(!open)}>
        <span className="caret">{open ? "▾" : "▸"}</span>
        {heading}
        {hv.link.kind === "offline" && <span className="guest-off" title={hv.link.why}>offline</span>}
        {hv.link.kind === "removed" && <span className="guest-off" data-testid="guest-removed-tag" title={hv.link.why}>removed</span>}
        {unseen > 0 && <span className="guest-badge" data-testid="guest-badge" aria-label={`${unseen} new`}>{unseen}</span>}
        <span className="count">{shares.length}</span>
      </div>
      {open && (
        <ul className="places">
          <RemovedRow hv={hv} />
          {shares.map((s) => <ShareRow key={s.id} hv={hv} s={s} selected={openShareId === s.id} />)}
          {projects.map((p) => (
            p.places.filter((pl) => !sharedSlugs.has(`${p.name}|${pl.slug}`)).map((pl) => (
              <NameRow key={`${p.id}|${pl.slug}`} hv={hv} project={p} slug={pl.slug} title={projects.length > 1 ? `${p.name} / ${pl.title || pl.slug}` : pl.title} />
            ))
          ))}
          {hv.shares === null && hv.link.kind !== "offline" && hv.link.kind !== "removed" && <li className="row guest-name"><span className="row-id"><span className="row-name guest-state">Connecting to {hv.host.name}…</span></span></li>}
        </ul>
      )}
    </div>
  );
}

/** Under a local project: each guest host's *<owner>'s Places* for it. */
export function GuestUnder({ projectName, localNames }: { projectName: string; localNames: readonly string[] }) {
  const g = useGuests();
  return (
    <>
      {g.hosts.map((hv) => {
        const p = placed(hv, projectName, localNames);
        if (!p.shares.length && !p.projects.length && !(p.status && hv.link.kind !== "up")) return null;
        return <HostGroup key={hv.host.id} hv={hv} shares={p.shares} projects={p.projects} heading={`${hv.owner}’s Places`} openShareId={g.open?.host === hv.host.id ? g.open.share : null} />;
      })}
    </>
  );
}

/** At the end of the sidebar: what matches no local project. */
export function SharedWithMe({ localNames }: { localNames: readonly string[] }) {
  const g = useGuests();
  if (!g.hosts.length) return null;
  // A host with no rows here shows its standing here only when this is where
  // its rows go (`placed`'s status): offline, it is not listed twice.
  const groups = g.hosts.map((hv) => ({ hv, ...placed(hv, null, localNames) }))
    .filter((x) => x.shares.length || x.projects.length || (x.status && x.hv.link.kind !== "up"));
  if (!groups.length) return null;
  return (
    <div className="project guest-project" data-testid="shared-with-me">
      <div className="project-h"><span className="pname">Shared with me</span></div>
      <div className="kids">
        {groups.map(({ hv, shares, projects }) => (
          <HostGroup key={hv.host.id} hv={hv} shares={shares} projects={projects} heading={`${hv.owner}’s Places`} openShareId={g.open?.host === hv.host.id ? g.open.share : null} />
        ))}
      </div>
    </div>
  );
}

/** The toasts: "David shared <slug> with you" (§6). */
export function GuestToasts() {
  const g = useGuests();
  if (!g.toasts.length) return null;
  return (
    <div className="guest-toasts" role="status" aria-live="polite" data-testid="guest-toasts">
      {g.toasts.map((t) => (
        <div key={t.id} className="guest-toast" data-testid="guest-toast">
          <span>{t.text}</span>
          <button className="guest-toast-x" aria-label="Dismiss" onClick={() => dismissToast(t.id)}>×</button>
        </div>
      ))}
    </div>
  );
}

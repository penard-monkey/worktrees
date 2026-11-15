// Design harness ONLY: three throwaway mocks of how a group-chat conversation
// starts and how a guest is invited (docs/proposals/group-chat-start.md).
// Nothing here is wired to the engine or to the app's state: each mock is a
// click-through storyboard drawn over the harness with the app's own classes
// and tokens, so a screenshot reads like the app. install.ts mounts it only
// for `?design=start-a|start-b|start-c` (`&as=guest` for the guest's side,
// `&theme=light` for a light pass).
//
//   start-a  Share from the session: one dialog invites a new person and
//            shares the session, Allow and Share kept as two acts.
//   start-b  A People page in Settings, out of Remote, beginner tier.
//   start-c  Conversation first: a Conversations list in the rail.
//
// Every control a person would press carries `data-act`, so the walk script
// (Playwright) presses them by name and counts. Components live at module
// scope (AGENTS.md: a component defined inside another remounts per render).
import { useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";

type Variant = "start-a" | "start-b" | "start-c";
type Side = "owner" | "guest";

const CSS = `
#ds-root { position: fixed; inset: 0; z-index: 10000; background: var(--bg-abyss); color: var(--txt); font-family: var(--font-ui); font-size: var(--fs-body); }
.ds-frame { display: grid; grid-template-columns: 44px 260px 1fr; height: 100%; }
.ds-rail { border-right: 1px solid var(--line); display: flex; flex-direction: column; align-items: center; gap: 14px; padding: 12px 0; background: var(--bg-tree); }
.ds-rail .ds-ri { width: 28px; height: 28px; border-radius: var(--r-md); display: grid; place-items: center; color: var(--txt-dim); font-size: 15px; position: relative; cursor: default; }
.ds-rail .ds-ri.on { color: var(--accent); background: var(--bg-elev); }
.ds-rail .ds-ri button { all: unset; width: 100%; height: 100%; display: grid; place-items: center; cursor: pointer; }
.ds-rail .ds-badge { position: absolute; right: -4px; top: -4px; background: var(--accent); color: var(--accent-ink); font-size: 9px; border-radius: 8px; padding: 0 4px; font-weight: 700; }
.ds-rail .ds-sp { flex: 1; }
.ds-side { border-right: 1px solid var(--line); background: var(--bg-tree); padding: 10px 8px; overflow: hidden; }
.ds-side h4 { margin: 10px 6px 4px; font-size: var(--fs-micro); letter-spacing: .08em; text-transform: uppercase; color: var(--txt-mute); font-weight: 700; }
.ds-row { display: flex; align-items: center; gap: 8px; padding: 5px 8px; border-radius: var(--r-md); color: var(--txt); font-size: var(--fs-row); }
.ds-row.sel { background: var(--bg-elev); color: var(--txt-hi); }
.ds-row .ds-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--idle); flex: none; }
.ds-row .ds-dot.live { background: var(--ok); }
.ds-row .ds-dot.share { background: var(--accent); }
.ds-row .ds-meta { margin-left: auto; color: var(--txt-mute); font-size: var(--fs-meta); }
.ds-row button { all: unset; cursor: pointer; display: contents; }
.ds-main { display: flex; flex-direction: column; min-width: 0; position: relative; }
.ds-head { display: flex; align-items: center; gap: 10px; height: 44px; padding: 0 14px; border-bottom: 1px solid var(--line); }
.ds-head b { color: var(--txt-hi); }
.ds-head .ds-crumb { color: var(--txt-mute); }
.ds-head .ds-sp { flex: 1; }
.ds-sub { display: flex; align-items: center; gap: 8px; padding: 6px 14px; color: var(--txt-dim); font-size: var(--fs-meta); }
.ds-body { flex: 1; overflow: hidden; padding: 16px 48px; display: flex; flex-direction: column; gap: 12px; }
.ds-bub { align-self: flex-end; max-width: 60%; background: var(--bg-elev); border: 1px solid var(--line); border-radius: var(--r-lg); padding: 10px 14px; }
.ds-rep { max-width: 80%; background: var(--bg-panel); border: 1px solid var(--line); border-radius: var(--r-lg); padding: 10px 14px; }
.ds-who { font-size: var(--fs-meta); color: var(--txt-dim); margin-bottom: 4px; }
.ds-strip { display: flex; align-items: center; gap: 8px; padding: 6px 14px; border-bottom: 1px solid var(--line); background: var(--bg-panel); font-size: var(--fs-meta); color: var(--txt-dim); }
.ds-strip .ds-pill { border: 1px solid var(--line-strong); border-radius: var(--r-full); padding: 1px 8px; color: var(--txt); }
.ds-menu { position: absolute; right: 12px; top: 40px; z-index: 5; background: var(--bg-elev); border: 1px solid var(--line-strong); border-radius: var(--r-md); box-shadow: var(--shadow-float); padding: 4px; min-width: 200px; }
.ds-menu button { all: unset; display: block; width: calc(100% - 20px); padding: 6px 10px; border-radius: var(--r-sm); cursor: pointer; color: var(--txt); }
.ds-menu button:hover, .ds-menu button.hot { background: var(--bg-panel); color: var(--txt-hi); }
.ds-menu hr { border: 0; border-top: 1px solid var(--line); margin: 4px 0; }
.ds-scrim { position: absolute; inset: 0; background: var(--scrim); display: flex; justify-content: center; align-items: flex-start; padding-top: 9vh; z-index: 20; }
.ds-modal { width: 560px; max-width: calc(100% - 32px); background: var(--bg-panel); border: 1px solid var(--line-strong); border-radius: var(--r-lg); box-shadow: var(--shadow-float); display: flex; flex-direction: column; }
.ds-modal header { display: flex; align-items: center; gap: 8px; padding: 14px 18px; border-bottom: 1px solid var(--line); }
.ds-modal header b { font-size: var(--fs-h); color: var(--txt-hi); }
.ds-modal header .ds-sp { flex: 1; }
.ds-steps { display: flex; gap: 6px; font-size: var(--fs-micro); color: var(--txt-mute); }
.ds-steps span.on { color: var(--accent); font-weight: 700; }
.ds-modal .ds-mb { padding: 16px 18px; display: flex; flex-direction: column; gap: 12px; }
.ds-modal footer { display: flex; align-items: center; justify-content: flex-end; gap: 8px; padding: 12px 18px; border-top: 1px solid var(--line); }
.ds-modal footer .ds-why { margin-right: auto; color: var(--txt-mute); font-size: var(--fs-meta); }
.ds-note { color: var(--txt-dim); font-size: var(--fs-meta); line-height: var(--lh-prose); }
.ds-line { line-height: var(--lh-prose); }
.ds-warn { border-left: 3px solid var(--warn); background: color-mix(in srgb, var(--warn) 10%, var(--bg-panel)); padding: 8px 12px; color: var(--txt-hi); line-height: var(--lh-prose); }
.ds-field { display: flex; flex-direction: column; gap: 4px; }
.ds-field span { font-size: var(--fs-meta); color: var(--txt-dim); }
.ds-field input, .ds-input { background: var(--bg-input); border: 1px solid var(--line-strong); border-radius: var(--r-md); color: var(--txt-hi); padding: 7px 10px; font: inherit; }
.ds-pick { display: flex; flex-direction: column; gap: 2px; border: 1px solid var(--line); border-radius: var(--r-md); padding: 4px; }
.ds-pick label, .ds-pick button { all: unset; display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: var(--r-sm); cursor: pointer; }
.ds-pick label:hover, .ds-pick button:hover { background: var(--bg-elev); }
.ds-pick .ds-new { color: var(--accent); }
.ds-pick .ds-sm { margin-left: auto; color: var(--txt-mute); font-size: var(--fs-meta); }
.ds-win { display: grid; grid-template-columns: 132px 1fr; gap: 14px; align-items: start; }
.ds-qr { width: 132px; height: 132px; background: #fff; border-radius: 6px; padding: 8px; box-sizing: border-box; }
.ds-qr i { display: block; width: 100%; height: 100%; background:
  linear-gradient(90deg,#000 10%,transparent 10% 20%,#000 20% 30%,transparent 30% 45%,#000 45% 55%,transparent 55% 70%,#000 70% 80%,transparent 80% 90%,#000 90%),
  linear-gradient(#000 10%,transparent 10% 25%,#000 25% 35%,transparent 35% 50%,#000 50% 60%,transparent 60% 75%,#000 75% 85%,transparent 85%);
  background-blend-mode: difference; }
.ds-link { display: flex; gap: 6px; }
.ds-link input { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: var(--fs-meta); }
.ds-code { font-family: var(--font-mono); font-size: 34px; letter-spacing: .12em; color: var(--txt-hi); text-align: center; padding: 6px 0; }
.ds-count { font-variant-numeric: tabular-nums; color: var(--txt-dim); font-size: var(--fs-meta); }
.ds-status { display: flex; gap: 8px; align-items: center; color: var(--txt-dim); font-size: var(--fs-meta); }
.ds-status i { width: 8px; height: 8px; border-radius: 50%; background: var(--warn); flex: none; }
.ds-status i.ok { background: var(--ok); }
.ds-reach { border: 1px solid var(--line); border-radius: var(--r-md); padding: 10px 12px; display: flex; flex-direction: column; gap: 8px; }
.ds-reach select { background: var(--bg-input); color: var(--txt-hi); border: 1px solid var(--line-strong); border-radius: var(--r-md); padding: 5px 8px; font: inherit; }
.ds-toast { position: absolute; right: 16px; bottom: 16px; z-index: 30; width: 340px; background: var(--bg-elev); border: 1px solid var(--line-strong); border-radius: var(--r-lg); box-shadow: var(--shadow-float); padding: 12px 14px; display: flex; flex-direction: column; gap: 8px; }
.ds-toast .ds-acts { display: flex; gap: 8px; justify-content: flex-end; }
.ds-settings { position: absolute; inset: 28px 48px; z-index: 15; background: var(--bg-panel); border: 1px solid var(--line-strong); border-radius: var(--r-lg); box-shadow: var(--shadow-float); display: grid; grid-template-rows: auto 1fr; overflow: hidden; }
.ds-settings > header { display: flex; align-items: center; gap: 10px; padding: 12px 18px; border-bottom: 1px solid var(--line); }
.ds-settings > header b { font-size: var(--fs-title); color: var(--txt-hi); }
.ds-settings > header .ds-sp { flex: 1; }
.ds-seg { display: flex; border: 1px solid var(--line-strong); border-radius: var(--r-md); overflow: hidden; font-size: var(--fs-meta); }
.ds-seg span { padding: 3px 10px; color: var(--txt-dim); }
.ds-seg span.on { background: var(--accent); color: var(--accent-ink); }
.ds-split { display: grid; grid-template-columns: 200px 1fr; min-height: 0; }
.ds-cats { border-right: 1px solid var(--line); padding: 10px; overflow: hidden; }
.ds-cats h5 { margin: 12px 8px 4px; font-size: var(--fs-micro); letter-spacing: .08em; color: var(--txt-mute); text-transform: uppercase; }
.ds-cats button { all: unset; display: block; padding: 5px 10px; border-radius: var(--r-md); color: var(--txt); cursor: pointer; width: calc(100% - 20px); }
.ds-cats button.on { background: var(--bg-elev); color: var(--accent); }
.ds-cats .ds-new-tag { font-size: 9px; color: var(--accent-ink); background: var(--accent); border-radius: 6px; padding: 0 4px; margin-left: 6px; font-weight: 700; }
.ds-page { padding: 18px 24px; overflow: auto; display: flex; flex-direction: column; gap: 16px; }
.ds-page h2 { margin: 0; font-size: var(--fs-title); color: var(--txt-hi); }
.ds-sec { display: flex; flex-direction: column; gap: 8px; }
.ds-sec > label { font-weight: var(--fw-bold); color: var(--txt-hi); }
.ds-check { display: flex; gap: 8px; align-items: flex-start; }
.ds-check input { margin-top: 3px; }
.ds-card { border: 1px solid var(--line); border-radius: var(--r-md); padding: 10px 12px; display: flex; align-items: center; gap: 10px; }
.ds-card .ds-sp { flex: 1; }
.ds-convo { display: grid; grid-template-columns: 1fr 320px; min-height: 0; flex: 1; }
.ds-log { padding: 14px 20px; display: flex; flex-direction: column; gap: 10px; overflow: hidden; }
.ds-ev { color: var(--txt-mute); font-size: var(--fs-meta); text-align: center; }
.ds-msg { background: var(--bg-panel); border: 1px solid var(--line); border-radius: var(--r-lg); padding: 8px 12px; max-width: 80%; }
.ds-msg.me { align-self: flex-end; background: var(--bg-elev); }
.ds-aside { border-left: 1px solid var(--line); padding: 14px; display: flex; flex-direction: column; gap: 10px; background: var(--bg-tree); }
.ds-aside h5 { margin: 4px 0 0; font-size: var(--fs-micro); letter-spacing: .08em; color: var(--txt-mute); text-transform: uppercase; }
.ds-composer { border-top: 1px solid var(--line); padding: 10px 14px; display: flex; gap: 8px; }
.ds-composer .ds-input { flex: 1; }
.ds-empty { margin: auto; text-align: center; display: flex; flex-direction: column; gap: 12px; align-items: center; max-width: 420px; color: var(--txt-dim); }
.ds-empty b { color: var(--txt-hi); font-size: var(--fs-h); }
.ds-label { position: absolute; left: 50%; transform: translateX(-50%); top: 6px; z-index: 40; font-size: var(--fs-micro); color: var(--txt-mute); letter-spacing: .06em; text-transform: uppercase; pointer-events: none; }
`;

// ── shared pieces ──────────────────────────────────────────────────────────

const Btn = ({ act, primary, danger, sm, disabled, onClick, children }: {
  act: string; primary?: boolean; danger?: boolean; sm?: boolean; disabled?: boolean; onClick?: () => void; children: ReactNode;
}) => (
  <button data-act={act} disabled={disabled} onClick={onClick}
    className={(primary ? "enter-btn" : "ctrl") + (sm ? " sm" : "") + (danger ? " danger" : "")}>{children}</button>
);

function Rail({ convo, onConvo, convoOn, badge }: { convo?: boolean; onConvo?: () => void; convoOn?: boolean; badge?: number }) {
  return (
    <div className="ds-rail">
      <div className="ds-ri on" title="Places">☰</div>
      {convo && (
        <div className={"ds-ri" + (convoOn ? " on" : "")} title="Conversations">
          <button data-act="rail-conversations" onClick={onConvo}>💬</button>
          {!!badge && <span className="ds-badge">{badge}</span>}
        </div>
      )}
      <div className="ds-sp" />
      <div className="ds-ri" title="Settings">⚙</div>
    </div>
  );
}

function OwnerSide({ shared }: { shared?: string }) {
  return (
    <div className="ds-side">
      <div className="ds-row">⌂ Home</div>
      <h4>worktrees</h4>
      <div className="ds-row"><span className="ds-dot" />(main)<span className="ds-meta">1d</span></div>
      <div className="ds-row sel"><span className={"ds-dot " + (shared ? "share" : "live")} />gc-lane<span className="ds-meta">{shared ? `shared · ${shared}` : "Codex"}</span></div>
      <div className="ds-row"><span className="ds-dot" />fix-login<span className="ds-meta">idle</span></div>
    </div>
  );
}

function GuestSide({ share, onOpen }: { share?: "new" | "waiting" | "live"; onOpen?: () => void }) {
  return (
    <div className="ds-side">
      <div className="ds-row">⌂ Home</div>
      <h4>worktrees</h4>
      <div className="ds-row"><span className="ds-dot" />(main)<span className="ds-meta">1d</span></div>
      {share && <>
        <h4>David's places</h4>
        <div className={"ds-row" + (share !== "new" ? " sel" : "")}>
          <button data-act="open-share" onClick={onOpen}><span className="ds-dot share" />gc-lane<span className="ds-meta">{share === "new" ? "shared with you · new" : share === "waiting" ? "waiting for David" : "live"}</span></button>
        </div>
        <div className="ds-row"><span className="ds-dot" />(main)<span className="ds-meta">Ask</span></div>
      </>}
    </div>
  );
}

function Lane({ children, sub, strip, menu }: { children?: ReactNode; sub?: ReactNode; strip?: ReactNode; menu?: ReactNode }) {
  return (
    <div className="ds-main">
      <div className="ds-head"><span className="ds-crumb">worktrees /</span><b>gc-lane</b><span className="ds-sp" /><span className="ds-crumb">● live</span>{menu}</div>
      {strip}
      <div className="ds-sub">Codex gpt-6.1-sol {sub}</div>
      <div className="ds-body">
        <div className="ds-bub">Add <code>--json</code> to <code>worktrees status</code>: plan first, then a stub and a test.</div>
        <div className="ds-rep"><div className="ds-who">Codex · 4:11 PM</div>I wrote the plan to <code>docs/notes/status-json.md</code>: exit 1 and a JSON error object for a missing worktree, text output unchanged.</div>
      </div>
      {children}
    </div>
  );
}

function GuestReading({ waiting }: { waiting?: boolean }) {
  return (
    <div className="ds-main">
      <div className="ds-head"><span className="ds-crumb">David's places /</span><b>gc-lane</b><span className="ds-sp" /><span className="ds-crumb">{waiting ? "waiting" : "● live · read-only"}</span></div>
      {waiting ? (
        <div className="ds-empty"><b>Waiting for David</b><span>David lets you in from his app. Nothing you open here reaches his agent.</span></div>
      ) : (
        <div className="ds-convo">
          <div className="ds-body">
            <div className="ds-bub">Add <code>--json</code> to <code>worktrees status</code>: plan first, then a stub and a test.</div>
            <div className="ds-rep"><div className="ds-who">Codex · 4:11 PM</div>I wrote the plan to <code>docs/notes/status-json.md</code>…</div>
          </div>
          <div className="ds-aside"><h5>Thread</h5><div className="ds-ev">Bret joined</div><div className="ds-composer"><input className="ds-input" placeholder="Write to David…" /></div></div>
        </div>
      )}
    </div>
  );
}

function Modal({ title, steps, step, children, foot, count }: { title: string; steps?: string[]; step?: number; children: ReactNode; foot: ReactNode; count?: string }) {
  return (
    <div className="ds-scrim">
      <div className="ds-modal" role="dialog" aria-label={title}>
        <header>
          <b>{title}</b><span className="ds-sp" />
          {count && <span className="ds-count">{count}</span>}
          {steps && <span className="ds-steps">{steps.map((s, i) => <span key={s} className={i === step ? "on" : ""}>{i + 1}. {s}</span>)}</span>}
        </header>
        <div className="ds-mb">{children}</div>
        <footer>{foot}</footer>
      </div>
    </div>
  );
}

/** Reaching this computer, inline: remote access is off and a person is being
 *  invited, so the invite asks for the one address, preselected, in words. */
const Reach = () => (
  <div className="ds-reach" data-testid="ds-reach">
    <div className="ds-line"><b>Bret's Mac has to reach this one.</b> Remote access is off. Turning it on listens on one private address, and only a paired key gets in.</div>
    <select aria-label="Address" defaultValue="ts">
      <option value="ts">100.101.1.2 · Tailscale (works from anywhere on your tailnet)</option>
      <option value="lan">10.211.55.15 · this Wi-Fi network only</option>
    </select>
    <div className="ds-note">Change or turn it off any time in Settings → Remote. Quitting worktrees turns it off.</div>
  </div>
);

const Window = ({ name, onCopy }: { name: string; onCopy: () => void }) => (
  <>
    <div className="ds-win">
      <div className="ds-qr"><i /></div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <div className="ds-line">Send {name} this link. In worktrees they choose <b>Join with a link</b> and paste it.</div>
        <div className="ds-link"><input className="ds-input" readOnly value="worktrees-pair:v1?addr=100.101.1.2:49350&pin=8f3a…" /><Btn act="copy-link" sm onClick={onCopy}>Copy</Btn></div>
        <div className="ds-status"><i />Waiting for {name}. One attempt; keep this open.</div>
      </div>
    </div>
    <div className="ds-note">One pairing window at a time: opening another closes this one, and this link then stops working.</div>
  </>
);

const Code = ({ name }: { name: string }) => (
  <>
    <div className="ds-line"><b>{name}'s MacBook</b> is asking to pair, from 100.101.4.7.</div>
    <div className="ds-code" data-testid="ds-code">482 913</div>
    <div className="ds-line"><b>Allow only if {name} reads you this same code from their screen.</b> The name is the Mac's own claim; the code is what shows it is {name}.</div>
    <div className="ds-note">Allow pairs {name}'s Mac as a guest. It shares nothing yet.</div>
  </>
);

const WARNING = "Bret can read this whole session from its first turn, including anything it printed. This can't be recalled.";

// ── A: share from the session ──────────────────────────────────────────────

function OwnerA() {
  const [s, set] = useState("lane");
  const [menu, setMenu] = useState(false);
  const [name, setName] = useState("");
  const steps = ["Who", "Pair", "Share"];
  const shareBtn = <Btn act="share-open" sm onClick={() => set("who")}>Share…</Btn>;
  const menuBtn = (
    <span style={{ position: "relative" }}>
      <Btn act="header-menu" sm onClick={() => setMenu(!menu)}>⋯</Btn>
      {menu && <div className="ds-menu"><button className="hot" data-act="menu-share" onClick={() => { setMenu(false); set("who"); }}>Share this session…</button><hr /><button>Close session</button><button>Switch agent…</button><button>Name this place…</button></div>}
    </span>
  );
  const shared = s === "shared" || s === "ask" || s === "reading";
  return (
    <div className="ds-frame">
      <Rail />
      <OwnerSide shared={shared ? "Bret" : undefined} />
      <Lane menu={menuBtn} sub={shared ? <span>· Shared with Bret {s === "reading" ? "· reading" : "· not opened yet"} <Btn act="thread" sm>Thread</Btn></span> : shareBtn}>
        {s === "who" && (
          <Modal title="Share gc-lane" steps={steps} step={0} foot={<>
            <span className="ds-why">{name.trim() ? "" : "Type their name first."}</span>
            <Btn act="cancel">Cancel</Btn>
            <Btn act="invite" primary disabled={!name.trim()} onClick={() => set("reach")}>Invite {name.trim() || "them"}</Btn>
          </>}>
            <div className="ds-line">Who should read this session?</div>
            <div className="ds-pick">
              <div className="ds-note" style={{ padding: "6px 8px" }}>Nobody is paired with this computer yet.</div>
              <label className="ds-new">＋ Someone new</label>
            </div>
            <label className="ds-field"><span>Their name, as you and the thread will see it</span>
              <input data-act="name" value={name} placeholder="Bret" onChange={(e) => setName(e.target.value)} /></label>
            <div className="ds-note">They pair their Mac with yours once, with a link you send them and a code you compare. Then you choose what they read.</div>
          </Modal>
        )}
        {s === "reach" && (
          <Modal title="Share gc-lane" steps={steps} step={1} foot={<><Btn act="cancel">Cancel</Btn><Btn act="show-link" primary onClick={() => set("window")}>Turn on and show the link</Btn></>}>
            <Reach />
          </Modal>
        )}
        {s === "window" && (
          <Modal title="Share gc-lane" steps={steps} step={1} count="Closes in 1:52" foot={<><Btn act="cancel">Cancel pairing</Btn></>}>
            <Window name="Bret" onCopy={() => set("code")} />
          </Modal>
        )}
        {s === "code" && (
          <Modal title="Share gc-lane" steps={steps} step={1} foot={<><Btn act="deny">Deny</Btn><Btn act="allow" primary onClick={() => set("share")}>Allow</Btn></>}>
            <Code name="Bret" />
          </Modal>
        )}
        {s === "share" && (
          <Modal title="Share gc-lane" steps={steps} step={2} foot={<><Btn act="not-now">Not now</Btn><Btn act="share" primary onClick={() => set("shared")}>Share with Bret</Btn></>}>
            <div className="ds-status"><i className="ok" />Bret's MacBook is paired as Bret, a guest.</div>
            <div className="ds-pick"><label><input type="checkbox" defaultChecked readOnly /> Bret <span className="ds-sm">just paired</span></label></div>
            <div className="ds-warn" data-testid="ds-warning">{WARNING}</div>
            <div className="ds-note">They ask to join when they open it, and you let them in. What they write sits in a thread beside the session; the agent sees none of it unless you send it.</div>
          </Modal>
        )}
        {s === "ask" && (
          <div className="ds-toast"><div><b>Bret asks to join gc-lane</b></div><div className="ds-note">From Bret's MacBook.</div>
            <div className="ds-acts"><Btn act="not-yet" sm>Not yet</Btn><Btn act="allow-join" primary sm onClick={() => set("reading")}>Allow</Btn></div></div>
        )}
      </Lane>
      {s === "shared" && <AutoAdvance to={() => set("ask")} />}
    </div>
  );
}

/** A beat for the other person's act (Bret opening the share). */
function AutoAdvance({ to }: { to: () => void }) {
  useState(() => window.setTimeout(to, 600));
  return null;
}

function GuestHome({ onJoin }: { onJoin?: () => void }) {
  return (
    <div className="ds-main">
      <div className="ds-empty">
        <b>worktrees</b>
        <span>a place for every work stream</span>
        <div style={{ display: "flex", gap: 8 }}><Btn act="new-project" primary>＋ New project…</Btn>{onJoin && <Btn act="join-link" onClick={onJoin}>Join with a link…</Btn>}</div>
        {onJoin && <span className="ds-note">Someone sent you a <code>worktrees-pair:</code> link? Join with it to read what they share.</span>}
      </div>
    </div>
  );
}

function JoinDialog({ onJoined }: { onJoined: () => void }) {
  const [phase, setPhase] = useState<"paste" | "code">("paste");
  const [link, setLink] = useState("");
  return phase === "paste" ? (
    <Modal title="Join with a link" foot={<><Btn act="cancel">Cancel</Btn><Btn act="join" primary disabled={!link.trim()} onClick={() => setPhase("code")}>Join</Btn></>}>
      <label className="ds-field"><span>The link they sent you</span>
        <input data-act="paste" value={link} placeholder="worktrees-pair:v1?addr=…" onChange={(e) => setLink(e.target.value)} /></label>
      <label className="ds-field"><span>This Mac's name, as they will see it</span><input defaultValue="Bret's MacBook" /></label>
      <div className="ds-note">Joining pairs this Mac as a guest: you read only what they share, after they let you in. Their computer is never driven from here. Only a private address (your network, Tailscale) is accepted.</div>
    </Modal>
  ) : (
    <Modal title="Join with a link" foot={<><Btn act="cancel">Cancel</Btn></>}>
      <div className="ds-line">Read this code to David. He presses Allow when his screen shows the same.</div>
      <div className="ds-code">482 913</div>
      <div className="ds-status"><i />Waiting for David to allow…</div>
      <AutoAdvance to={onJoined} />
    </Modal>
  );
}

function GuestA() {
  const [s, set] = useState<"home" | "join" | "toast" | "waiting" | "reading">("home");
  return (
    <div className="ds-frame">
      <Rail />
      <GuestSide share={s === "toast" ? "new" : s === "waiting" ? "waiting" : s === "reading" ? "live" : undefined} onOpen={() => set("waiting")} />
      {s === "home" || s === "join" ? <GuestHome onJoin={() => set("join")} /> : s === "toast" ? <GuestHome onJoin={() => {}} /> : <GuestReading waiting={s === "waiting"} />}
      {s === "join" && <div style={{ position: "absolute", inset: 0 }}><JoinDialog onJoined={() => set("toast")} /></div>}
      {s === "toast" && (
        <div className="ds-toast"><div><b>David shared gc-lane with you</b></div><div className="ds-note">A Codex session in worktrees. Opening it asks David to let you in.</div>
          <div className="ds-acts"><Btn act="open-toast" primary sm onClick={() => set("waiting")}>Open</Btn></div></div>
      )}
      {s === "waiting" && <AutoAdvance to={() => set("reading")} />}
    </div>
  );
}

// ── B: a People page in Settings ───────────────────────────────────────────

function SettingsShell({ page, onPage, people, children }: { page: string; onPage: (p: string) => void; people: boolean; children: ReactNode }) {
  const cat = (id: string, label: string, tag?: boolean) => (
    <button data-act={`cat-${id}`} className={page === id ? "on" : ""} onClick={() => onPage(id)}>{label}{tag && <span className="ds-new-tag">new</span>}</button>
  );
  return (
    <div className="ds-settings">
      <header><b>Settings</b><span className="ds-sp" /><span className="ds-seg"><span className="on">Beginner</span><span>Advanced</span><span>Expert</span></span><Btn act="close-settings" sm onClick={() => onPage("closed")}>✕</Btn></header>
      <div className="ds-split">
        <nav className="ds-cats">
          <h5>Workspace</h5>{cat("general", "General")}{cat("appearance", "Appearance")}
          {people && <><h5>People</h5>{cat("people", "People", true)}</>}
          <h5>Agents</h5>{cat("agents", "Agents")}
          <h5>Remote access</h5>{cat("remote", "Remote")}
          <h5>System</h5>{cat("updates", "Updates")}
        </nav>
        <div className="ds-page">{children}</div>
      </div>
    </div>
  );
}

function BetaSection({ on, onTick }: { on: boolean; onTick: () => void }) {
  return (
    <div className="ds-sec">
      <label>Beta features</label>
      <label className="ds-check"><input type="checkbox" readOnly checked={false} /> <span>Ideas<div className="ds-note">Adds the Ideas window.</div></span></label>
      <label className="ds-check"><input data-act="beta-group-chat" type="checkbox" checked={on} onChange={onTick} /> <span>Group chat<div className="ds-note">Share a running session with someone you work with, and talk beside it. Adds Settings → People.</div></span></label>
    </div>
  );
}

function PeoplePage({ side, paired, shared, onInvite, onJoin, onShare }: { side: Side; paired: boolean; shared: boolean; onInvite: () => void; onJoin: () => void; onShare: () => void }) {
  return (
    <>
      <h2>People</h2>
      <div className="ds-note">The people you work with on this computer, and the computers you are a guest of. A guest reads only what you share, after you let them in; nothing they write reaches an agent unless you send it.</div>
      {side === "owner" ? (
        <>
          <div className="ds-sec"><label>Your name, as they see it</label><input className="ds-input" defaultValue="David" style={{ maxWidth: 260 }} /></div>
          <div className="ds-sec"><label>People who can read what you share</label>
            {!paired ? <div className="ds-note">Nobody yet.</div> : (
              <div className="ds-card"><b>Bret</b><span className="ds-note">Bret's MacBook · paired just now · {shared ? "reads gc-lane" : "nothing shared"}</span><span className="ds-sp" />
                {!shared && <Btn act="share-a-session" sm onClick={onShare}>Share a session…</Btn>}<Btn act="remove" sm danger>Remove</Btn></div>
            )}
            <div><Btn act="invite-someone" primary sm onClick={onInvite}>Invite someone…</Btn></div>
          </div>
          <div className="ds-sec"><label>What you share</label>
            {shared ? <div className="ds-card"><span className="ds-dot" />gc-lane · Codex<span className="ds-sp" /><span className="ds-note">with Bret</span><Btn act="stop" sm>Stop sharing…</Btn></div> : <div className="ds-note">Nothing is shared. Share from a session (Share…), or from a person above.</div>}
          </div>
          <div className="ds-sec"><label>Show place names to guests</label><label className="ds-check"><input type="checkbox" readOnly checked /> worktrees</label></div>
        </>
      ) : (
        <>
          <div className="ds-sec"><label>Computers you are a guest of</label>
            {paired ? <div className="ds-card"><b>David's Mac</b><span className="ds-note">paired just now · what David shares shows in the sidebar under David's places</span></div> : <div className="ds-note">None yet.</div>}
            <div><Btn act="join-link" primary sm onClick={onJoin}>Join with a link…</Btn></div>
          </div>
          <div className="ds-sec"><label>People who can read what you share</label><div className="ds-note">Nobody yet. <span style={{ color: "var(--accent)" }}>Invite someone…</span></div></div>
        </>
      )}
    </>
  );
}

function OwnerB() {
  const [page, setPage] = useState("general");
  const [beta, setBeta] = useState(false);
  const [flow, setFlow] = useState<"none" | "name" | "window" | "code" | "pick" | "warn">("none");
  const [paired, setPaired] = useState(false);
  const [shared, setShared] = useState(false);
  const [ask, setAsk] = useState(false);
  const [reading, setReading] = useState(false);
  const [name, setName] = useState("");
  const steps = ["Name", "Pair"];
  return (
    <div className="ds-frame">
      <Rail />
      <OwnerSide shared={shared ? "Bret" : undefined} />
      <Lane sub={shared ? <span>· Shared with Bret {reading ? "· reading" : ""}</span> : <Btn act="share-open" sm>Share…</Btn>}>
        {page !== "closed" && (
          <SettingsShell page={page} onPage={setPage} people={beta}>
            {page === "general" && <><h2>General</h2><div className="ds-sec"><label>Startup</label><label className="ds-check"><input type="checkbox" readOnly /> Restore last place on launch</label></div><BetaSection on={beta} onTick={() => setBeta(!beta)} /></>}
            {page === "people" && <PeoplePage side="owner" paired={paired} shared={shared} onInvite={() => setFlow("name")} onJoin={() => {}} onShare={() => setFlow("pick")} />}
          </SettingsShell>
        )}
        {flow === "name" && (
          <Modal title="Invite someone" steps={steps} step={0} foot={<><Btn act="cancel" onClick={() => setFlow("none")}>Cancel</Btn><Btn act="show-link" primary disabled={!name.trim()} onClick={() => setFlow("window")}>Turn on and show the link</Btn></>}>
            <label className="ds-field"><span>Their name, as you and the thread will see it</span><input data-act="name" value={name} placeholder="Bret" onChange={(e) => setName(e.target.value)} /></label>
            <Reach />
          </Modal>
        )}
        {flow === "window" && <Modal title="Invite someone" steps={steps} step={1} count="Closes in 1:52" foot={<Btn act="cancel">Cancel pairing</Btn>}><Window name="Bret" onCopy={() => setFlow("code")} /></Modal>}
        {flow === "code" && <Modal title="Invite someone" steps={steps} step={1} foot={<><Btn act="deny">Deny</Btn><Btn act="allow" primary onClick={() => { setPaired(true); setFlow("none"); }}>Allow</Btn></>}><Code name="Bret" /></Modal>}
        {flow === "pick" && (
          <Modal title="Share a session with Bret" foot={<><Btn act="cancel" onClick={() => setFlow("none")}>Cancel</Btn></>}>
            <div className="ds-line">Which session should Bret read? Running Claude and Codex sessions:</div>
            <div className="ds-pick">
              <button data-act="pick-gc-lane" onClick={() => setFlow("warn")}><span className="ds-dot" />gc-lane<span className="ds-sm">worktrees · Codex · live</span></button>
              <button><span className="ds-dot" />billing-refactor<span className="ds-sm">garden · Claude · live</span></button>
            </div>
          </Modal>
        )}
        {flow === "warn" && (
          <Modal title="Share gc-lane with Bret" foot={<><Btn act="cancel" onClick={() => setFlow("none")}>Cancel</Btn><Btn act="share" primary onClick={() => { setShared(true); setFlow("none"); setPage("closed"); window.setTimeout(() => setAsk(true), 600); }}>Share</Btn></>}>
            <div className="ds-warn">{WARNING}</div>
            <div className="ds-note">They ask to join when they open it, and you let them in.</div>
          </Modal>
        )}
        {ask && !reading && <div className="ds-toast"><div><b>Bret asks to join gc-lane</b></div><div className="ds-acts"><Btn act="allow-join" primary sm onClick={() => setReading(true)}>Allow</Btn></div></div>}
      </Lane>
    </div>
  );
}

function GuestB() {
  const [page, setPage] = useState("general");
  const [beta, setBeta] = useState(false);
  const [join, setJoin] = useState(false);
  const [paired, setPaired] = useState(false);
  const [open, setOpen] = useState<"none" | "waiting" | "reading">("none");
  const closed = page === "closed";
  return (
    <div className="ds-frame">
      <Rail />
      <GuestSide share={paired ? (open === "none" ? "new" : open === "waiting" ? "waiting" : "live") : undefined} onOpen={() => setOpen("waiting")} />
      {open === "none" ? <GuestHome /> : <GuestReading waiting={open === "waiting"} />}
      {open === "waiting" && <AutoAdvance to={() => setOpen("reading")} />}
      {!closed && (
        <div style={{ position: "absolute", inset: 0, left: 304 }}>
          <SettingsShell page={page} onPage={setPage} people={beta}>
            {page === "general" && <><h2>General</h2><BetaSection on={beta} onTick={() => setBeta(!beta)} /></>}
            {page === "people" && <PeoplePage side="guest" paired={paired} shared={false} onInvite={() => {}} onJoin={() => setJoin(true)} onShare={() => {}} />}
          </SettingsShell>
          {join && <JoinDialog onJoined={() => { setJoin(false); setPaired(true); }} />}
        </div>
      )}
    </div>
  );
}

// ── C: conversation first ──────────────────────────────────────────────────

function ConvoList({ items, onNew, onJoin, onOpen, side }: { items: { title: string; who: string; unread?: boolean }[]; onNew?: () => void; onJoin?: () => void; onOpen?: () => void; side: Side }) {
  return (
    <div className="ds-side">
      <h4>Conversations</h4>
      {items.map((c) => (
        <div key={c.title} className={"ds-row" + (c.unread ? "" : " sel")}><button data-act="open-convo" onClick={onOpen}><span className={"ds-dot " + (c.unread ? "share" : "")} />{c.title}<span className="ds-meta">{c.who}</span></button></div>
      ))}
      {items.length === 0 && <div className="ds-note" style={{ padding: "4px 8px" }}>None yet.</div>}
      <div style={{ padding: "10px 6px", display: "flex", gap: 6, flexWrap: "wrap" }}>
        {side === "owner" ? <Btn act="new-convo" sm primary onClick={onNew}>New conversation</Btn> : null}
        <Btn act="join-link" sm onClick={onJoin}>Join with a link…</Btn>
      </div>
    </div>
  );
}

function OwnerC() {
  const [view, setView] = useState<"places" | "convos">("places");
  const [s, set] = useState<"empty" | "new" | "person" | "window" | "code" | "attach" | "warn" | "open">("empty");
  const [title, setTitle] = useState("");
  const [name, setName] = useState("");
  const [people, setPeople] = useState<string[]>([]);
  const [attached, setAttached] = useState(false);
  const [joinAsk, setJoinAsk] = useState(false);
  const [reading, setReading] = useState(false);
  const items = s === "open" ? [{ title: title || "status --json", who: "Bret" }] : [];
  return (
    <div className="ds-frame">
      <Rail convo convoOn={view === "convos"} onConvo={() => setView("convos")} />
      {view === "places" ? <OwnerSide /> : <ConvoList side="owner" items={items} onNew={() => set("new")} />}
      {view === "places" ? <Lane sub={null} /> : (
        <div className="ds-main">
          {s === "open" ? (
            <>
              <div className="ds-head"><b>{title || "status --json"}</b><span className="ds-sp" /><span className="ds-crumb">David, Bret</span></div>
              <div className="ds-convo">
                <div style={{ display: "flex", flexDirection: "column", minHeight: 0 }}>
                  <div className="ds-log">
                    <div className="ds-ev">David started the conversation · Bret was invited · gc-lane was shared with Bret</div>
                    {reading && <div className="ds-ev">Bret joined and is reading gc-lane</div>}
                    <div className="ds-msg me">Bret, the plan for <code>--json</code> is in the session. Tell me what's missing.</div>
                  </div>
                  <div className="ds-composer"><input className="ds-input" placeholder="Write to Bret…" /></div>
                </div>
                <div className="ds-aside">
                  <h5>People</h5><div className="ds-row"><span className="ds-dot live" />Bret<span className="ds-meta">{reading ? "reading" : "invited"}</span></div>
                  <h5>Sessions</h5><div className="ds-card"><span className="ds-dot live" />gc-lane<span className="ds-sp" /><span className="ds-note">Codex · live</span></div>
                  <Btn act="attach-more" sm>Attach a session…</Btn>
                </div>
              </div>
              {joinAsk && !reading && <div className="ds-toast"><div><b>Bret asks to join gc-lane</b></div><div className="ds-acts"><Btn act="allow-join" primary sm onClick={() => setReading(true)}>Allow</Btn></div></div>}
            </>
          ) : (
            <div className="ds-empty"><b>Talk about work with someone</b><span>A conversation has a topic, the people in it, and the sessions you choose to show them.</span><Btn act="new-convo-empty" primary onClick={() => set("new")}>New conversation</Btn></div>
          )}
          {s === "new" && (
            <Modal title="New conversation" foot={<><Btn act="cancel" onClick={() => set("empty")}>Cancel</Btn><Btn act="start" primary disabled={!title.trim() || people.length === 0} onClick={() => { set("open"); window.setTimeout(() => setJoinAsk(true), 600); }}>{attached ? "Start, and share gc-lane with Bret" : "Start"}</Btn></>}>
              <label className="ds-field"><span>What is it about?</span><input data-act="title" value={title} placeholder="status --json" onChange={(e) => setTitle(e.target.value)} /></label>
              <div className="ds-field"><span>People</span>
                <div className="ds-pick">{people.map((p) => <label key={p}>{p}<span className="ds-sm">paired</span></label>)}
                  <button className="ds-new" data-act="add-person" onClick={() => set("person")}>＋ Someone new</button></div></div>
              <div className="ds-field"><span>Sessions they can read (optional)</span>
                <div className="ds-pick">{attached ? <label>gc-lane<span className="ds-sm">Codex · live</span></label> : null}
                  <button className="ds-new" data-act="attach" onClick={() => set("attach")}>＋ Attach a running session</button></div></div>
              {attached && <div className="ds-warn">{WARNING}</div>}
            </Modal>
          )}
          {s === "person" && (
            <Modal title="Add someone new" steps={["Name", "Pair"]} step={0} foot={<><Btn act="cancel" onClick={() => set("new")}>Cancel</Btn><Btn act="show-link" primary disabled={!name.trim()} onClick={() => set("window")}>Turn on and show the link</Btn></>}>
              <label className="ds-field"><span>Their name</span><input data-act="name" value={name} placeholder="Bret" onChange={(e) => setName(e.target.value)} /></label>
              <Reach />
            </Modal>
          )}
          {s === "window" && <Modal title="Add someone new" steps={["Name", "Pair"]} step={1} count="Closes in 1:52" foot={<Btn act="cancel">Cancel pairing</Btn>}><Window name="Bret" onCopy={() => set("code")} /></Modal>}
          {s === "code" && <Modal title="Add someone new" steps={["Name", "Pair"]} step={1} foot={<><Btn act="deny">Deny</Btn><Btn act="allow" primary onClick={() => { setPeople(["Bret"]); set("new"); }}>Allow</Btn></>}><Code name="Bret" /></Modal>}
          {s === "attach" && (
            <Modal title="Attach a session" foot={<Btn act="cancel" onClick={() => set("new")}>Cancel</Btn>}>
              <div className="ds-pick">
                <button data-act="pick-gc-lane" onClick={() => { setAttached(true); set("new"); }}><span className="ds-dot" />gc-lane<span className="ds-sm">worktrees · Codex · live</span></button>
                <button><span className="ds-dot" />billing-refactor<span className="ds-sm">garden · Claude · live</span></button>
              </div>
            </Modal>
          )}
        </div>
      )}
    </div>
  );
}

function GuestC() {
  const [view, setView] = useState<"places" | "convos">("places");
  const [join, setJoin] = useState(false);
  const [paired, setPaired] = useState(false);
  const [open, setOpen] = useState<"none" | "convo" | "waiting" | "reading">("none");
  const items = paired ? [{ title: "status --json", who: "David", unread: open === "none" }] : [];
  return (
    <div className="ds-frame">
      <Rail convo convoOn={view === "convos"} onConvo={() => setView("convos")} badge={paired && open === "none" ? 1 : 0} />
      {view === "places" ? <GuestSide /> : <ConvoList side="guest" items={items} onJoin={() => setJoin(true)} onOpen={() => setOpen("convo")} />}
      {view === "places" || open === "none" ? (
        <div className="ds-main"><div className="ds-empty"><b>Conversations</b><span>{paired ? "David added you to a conversation." : "Someone sent you a worktrees-pair: link? Join with it."}</span></div></div>
      ) : open === "convo" ? (
        <div className="ds-main">
          <div className="ds-head"><b>status --json</b><span className="ds-sp" /><span className="ds-crumb">David, Bret</span></div>
          <div className="ds-convo">
            <div className="ds-log"><div className="ds-ev">David added you · gc-lane is shared with you</div><div className="ds-msg">Bret, the plan for <code>--json</code> is in the session. Tell me what's missing.</div></div>
            <div className="ds-aside"><h5>Sessions</h5><div className="ds-card"><span className="ds-dot live" />gc-lane<span className="ds-sp" /><Btn act="open-session" sm primary onClick={() => setOpen("waiting")}>Open</Btn></div></div>
          </div>
        </div>
      ) : <GuestReading waiting={open === "waiting"} />}
      {open === "waiting" && <AutoAdvance to={() => setOpen("reading")} />}
      {join && <div style={{ position: "absolute", inset: 0 }}><JoinDialog onJoined={() => { setJoin(false); setPaired(true); }} /></div>}
    </div>
  );
}

// ── mount ──────────────────────────────────────────────────────────────────

const VIEWS: Record<Variant, Record<Side, () => ReactNode>> = {
  "start-a": { owner: () => <OwnerA />, guest: () => <GuestA /> },
  "start-b": { owner: () => <OwnerB />, guest: () => <GuestB /> },
  "start-c": { owner: () => <OwnerC />, guest: () => <GuestC /> },
};

export function mountStartDesign(): void {
  const q = new URLSearchParams(location.search);
  const v = q.get("design") as Variant;
  const side: Side = q.get("as") === "guest" ? "guest" : "owner";
  if (!VIEWS[v]) return;
  const style = document.createElement("style");
  style.textContent = CSS;
  document.head.appendChild(style);
  const el = document.createElement("div");
  el.id = "ds-root";
  el.dataset.design = v;
  el.dataset.side = side;
  document.body.appendChild(el);
  createRoot(el).render(<>{VIEWS[v][side]()}</>);
}

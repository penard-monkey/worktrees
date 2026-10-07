// The dock's Pull requests tab (docs/proposals/pull-requests.md §5).
//
// One rendering for every place of a project: the project's list, with the
// CURRENT place's PR pinned on top — on a lane it reads "mine, then everything
// else", on (main) it is simply the list. Four groups:
//
//   This place · In a place · No place (collapsed) · Recently merged (collapsed)
//
// "No place" is collapsed because it is the stale tail — other people's
// branches, month-old conflicts (§3's table). A row click opens the PR on
// GitHub; ⌥-click, or the row menu's "Go to place", selects its place.
// Read-only toward GitHub: nothing here writes anything anywhere.
//
// Module scope, props in — never defined inside App() (it would remount on
// every render and lose its open menu and collapsed groups).
import { useState, type MouseEvent, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { copyToClipboard } from "./clipboard";
import { CtxMenu } from "./CtxMenu";
import { ageOf, prSummary, type PrRow, type PrsReply } from "./prs";

type Props = {
  reply: PrsReply;
  slug: string;
  /** slug → display name, for the place a row belongs to */
  placeName: (slug: string) => string;
  onSelectPlace: (slug: string) => void;
  onError: (e: unknown) => void;
};

/** A command to run in a terminal, with a Copy button. The app shows these and
 *  never runs them: `brew install` is system-wide and `gh auth login` is
 *  interactive (§7). */
function CopyLine({ cmd, onError }: { cmd: string; onError: (e: unknown) => void }) {
  const [done, setDone] = useState(false);
  return (
    <div className="prs-cmd">
      <code>{cmd}</code>
      <button
        className="ctrl sm"
        data-track="prs.copy"
        onClick={() => copyToClipboard(cmd).then(() => { setDone(true); setTimeout(() => setDone(false), 1200); }, onError)}
      >
        {done ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

/** The one-line states. Each says what is wrong and the one thing that fixes it. */
function PrsEmpty({ reply, onError }: { reply: PrsReply; onError: (e: unknown) => void }) {
  const host = reply.host ?? "github.com";
  const login = host === "github.com" ? "gh auth login" : `gh auth login -h ${host}`;
  switch (reply.state) {
    case "gh_missing":
      return (
        <div className="prs-empty" data-testid="prs-empty" data-state={reply.state}>
          <div className="prs-empty-lead">The GitHub CLI is not installed.</div>
          <CopyLine cmd="brew install gh" onError={onError} />
          <div className="prs-empty-sub">Then sign in once:</div>
          <CopyLine cmd="gh auth login" onError={onError} />
          <button className="prs-link" onClick={() => openUrl("https://cli.github.com").catch(onError)}>About gh ↗</button>
        </div>
      );
    case "logged_out":
    case "no_host_token":
      return (
        <div className="prs-empty" data-testid="prs-empty" data-state={reply.state}>
          <div className="prs-empty-lead">
            {reply.state === "logged_out" ? "The GitHub CLI is not signed in." : `The GitHub CLI is not signed in to ${host}.`}
          </div>
          <div className="prs-empty-sub">Run this in a terminal:</div>
          <CopyLine cmd={login} onError={onError} />
        </div>
      );
    case "not_found":
      return (
        <div className="prs-empty" data-testid="prs-empty" data-state={reply.state}>
          <div className="prs-empty-lead">
            GitHub says this repository does not exist for the signed-in account
            {reply.viewer ? <> (<b>{reply.viewer}</b>)</> : null}.
          </div>
          <div className="prs-empty-sub">A private repository on another account looks the same — check which account is active:</div>
          <CopyLine cmd="gh auth status" onError={onError} />
        </div>
      );
    case "error":
      return (
        <div className="prs-empty" data-testid="prs-empty" data-state={reply.state}>
          <div className="prs-empty-lead">Couldn't reach GitHub.</div>
          {reply.message && <div className="prs-empty-sub">{reply.message}</div>}
        </div>
      );
    default:
      return null;
  }
}

function Row({ p, name, viewer, here, onMenu, onSelectPlace, onError }: {
  p: PrRow;
  name: string | null;
  viewer: string | null;
  here: boolean;
  onMenu: (e: MouseEvent, p: PrRow) => void;
  onSelectPlace: (slug: string) => void;
  onError: (e: unknown) => void;
}) {
  const age = ageOf(p.state === "open" ? p.updated_at : p.ended_at ?? p.updated_at);
  const who = p.author && p.author !== viewer ? p.author : null;
  return (
    <button
      className={"prs-row" + (p.draft || p.state === "closed" ? " dim" : "")}
      data-testid="prs-row"
      data-pr={p.number}
      title={`${prSummary(p)} — open on GitHub${p.place && !here ? " (⌥-click: go to its place)" : ""}`}
      onClick={(e) => {
        if (e.altKey && p.place && !here) onSelectPlace(p.place);
        else openUrl(p.url).catch(onError);
      }}
      onContextMenu={(e) => onMenu(e, p)}
    >
      <span className={"pr-dot " + p.tone} aria-hidden="true" />
      <span className="prs-main">
        <span className="prs-title"><span className="prs-num">#{p.number}</span> {p.title}</span>
        <span className="prs-meta">
          {/* the place, when one owns it — what makes the list worth keeping
              beside the nav; otherwise the head branch */}
          <span className="prs-where">{name ?? p.head}</span>
          <span className="prs-sep" aria-hidden="true"> · </span>
          <span>{p.label}</span>
          {who && <><span className="prs-sep" aria-hidden="true"> · </span><span>{who}</span></>}
          {age && <><span className="prs-sep" aria-hidden="true"> · </span><span>{age}</span></>}
        </span>
      </span>
    </button>
  );
}

function Group({ title, rows, open, onToggle, children }: {
  title: string;
  rows: number;
  open: boolean;
  onToggle?: () => void;
  children: ReactNode;
}) {
  return (
    <section className="prs-group" data-testid="prs-group" data-open={open ? "1" : "0"}>
      {onToggle ? (
        <button className="prs-group-head" onClick={onToggle} aria-expanded={open}>
          <span>{title} ({rows})</span>
          <span className="prs-group-toggle">{open ? "▾ hide" : "▸ show"}</span>
        </button>
      ) : (
        <div className="prs-group-head static"><span>{title}{rows > 1 ? ` (${rows})` : ""}</span></div>
      )}
      {open && children}
    </section>
  );
}

export function PrsPane({ reply, slug, placeName, onSelectPlace, onError }: Props) {
  const [showNoPlace, setShowNoPlace] = useState(false);
  const [showRecent, setShowRecent] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; p: PrRow } | null>(null);

  if (reply.state !== "ok" || !reply.view) return <PrsEmpty reply={reply} onError={onError} />;
  const v = reply.view;
  const mine = [...v.open, ...v.recent].find((p) => p.place === slug) ?? null;
  const inPlace = v.open.filter((p) => p.place && p !== mine);
  const noPlace = v.open.filter((p) => !p.place);
  const recent = v.recent.filter((p) => p !== mine);
  const onMenu = (e: MouseEvent, p: PrRow) => { e.preventDefault(); setMenu({ x: e.clientX, y: e.clientY, p }); };
  const row = (p: PrRow) => (
    <Row key={p.number} p={p} name={p.place ? placeName(p.place) : null} viewer={v.viewer}
      here={p.place === slug} onMenu={onMenu} onSelectPlace={onSelectPlace} onError={onError} />
  );

  return (
    <div className="prs-pane" data-testid="prs-pane">
      {reply.stale && reply.fetched_at && (
        <div className="prs-stale" title={reply.message ?? undefined}>
          Offline — as of {new Date(reply.fetched_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
        </div>
      )}
      {mine && <Group title="This place" rows={1} open>{row(mine)}</Group>}
      {inPlace.length > 0 && <Group title="In a place" rows={inPlace.length} open>{inPlace.map(row)}</Group>}
      {noPlace.length > 0 && (
        <Group title="No place" rows={noPlace.length} open={showNoPlace} onToggle={() => setShowNoPlace((o) => !o)}>
          {noPlace.map(row)}
        </Group>
      )}
      {recent.length > 0 && (
        <Group title="Recently merged" rows={recent.length} open={showRecent} onToggle={() => setShowRecent((o) => !o)}>
          {recent.map(row)}
        </Group>
      )}
      {!mine && v.open.length === 0 && recent.length === 0 && (
        <div className="prs-empty" data-testid="prs-empty" data-state="none">
          <div className="prs-empty-lead">No open pull requests.</div>
        </div>
      )}
      {menu && (
        <CtxMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          <button className="pop-item" onClick={() => { openUrl(menu.p.url).catch(onError); setMenu(null); }}>Open on GitHub</button>
          {menu.p.place && menu.p.place !== slug && (
            <button className="pop-item" onClick={() => { onSelectPlace(menu.p.place!); setMenu(null); }}>Go to place</button>
          )}
          <button className="pop-item" onClick={() => { copyToClipboard(menu.p.url).catch(onError); setMenu(null); }}>Copy link</button>
        </CtxMenu>
      )}
    </div>
  );
}

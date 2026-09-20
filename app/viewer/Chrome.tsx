// The staleness header.
//
// This is the reason the feature exists. `docs/proposals/place-docs.md` §1.1
// measured eleven live places on one project, seven of them carrying a docs
// tree that main had already restructured — `ssdlc` 53 commits behind, its ADRs
// reading as current. A viewer that shows "the docs" without saying WHICH
// PLACE, and how far behind what, is not neutral; it is silently wrong, and it
// is wrong at the moment it costs the most.
//
// Two decisions, both load-bearing:
//
// CHROME, NEVER CONTENT. It is built from `meta` into DOM the markdown pipeline
// cannot produce — `markdown.tsx` turns raw HTML into a text node, so no
// document can emit an element, let alone one carrying `data-chrome`. A header
// rendered as markdown could be forged by the document it describes: a repo
// could open with a convincing "up to date with main" blockquote and the reader
// would have no way to tell the tool's word from the repo's.
//
// STICKY, NEVER A BADGE. "Unmissable" means it survives scrolling — a banner at
// the top of a document is gone after one page-down, on exactly the document
// being read 53 commits late.
//
// Both timestamps are printed WHEN THEY DIFFER, and only then. `derived_epoch`
// is when this text was derived; `status_epoch` is when the git measurement
// behind "53 behind origin/main" was taken. A re-derive refreshes the first
// without re-running the second, so collapsing them into one line would assert
// that the staleness numbers are as fresh as the prose. When they agree there
// is nothing to disambiguate and a second timestamp is just noise.
import { useEffect, useState } from "react";
import type { Meta } from "./contract";

/** Seconds → "just now" / "42s ago" / "7m ago" / "3h ago" / "2d ago". */
export function ago(epoch: number, now: number): string {
  if (!epoch) return "unknown";
  const d = Math.max(0, Math.floor(now - epoch));
  if (d < 2) return "just now";
  if (d < 60) return `${d}s ago`;
  if (d < 3600) return `${Math.floor(d / 60)}m ago`;
  if (d < 86400) return `${Math.floor(d / 3600)}h ago`;
  return `${Math.floor(d / 86400)}d ago`;
}

const clock = (epoch: number): string =>
  epoch ? new Date(epoch * 1000).toLocaleTimeString(undefined, { hour12: false }) : "—";

/** Worst-of. Drives the dot's hue and the band's tint — never the words. */
export function staleness(meta: Meta): "clean" | "dirty" | "unknown" {
  if (meta.behind < 0 || meta.dirty < 0 || meta.branch === "?") return "unknown";
  return meta.behind === 0 && meta.dirty === 0 ? "clean" : "dirty";
}

export function Chrome({ meta, nav }: { meta: Meta | null; nav: React.ReactNode }) {
  // Ticks so the ages keep counting. If the server stops answering, the header
  // is the thing that visibly grows old — silence has to look like something.
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => clearInterval(t);
  }, []);

  if (!meta) {
    return (
      <header className="chrome chrome-unknown" data-chrome="1">
        <div className="chrome-row">
          <span className="chrome-dot" data-state="unknown" />
          <span className="chrome-place">place unknown</span>
          <span className="chrome-warn">
            this response carried no <code>meta</code> — nothing here can tell you which worktree you are reading
          </span>
        </div>
        <div className="chrome-row chrome-nav">{nav}</div>
      </header>
    );
  }

  const state = staleness(meta);
  const behind =
    meta.behind < 0
      ? "distance unknown"
      : meta.behind === 0
        ? `up to date with ${meta.base}`
        : `${meta.behind} behind ${meta.base}`;
  const dirty = meta.dirty < 0 ? "dirty count unknown" : meta.dirty === 0 ? "clean tree" : `${meta.dirty} dirty`;
  const split = meta.status_epoch !== 0 && meta.status_epoch !== meta.derived_epoch;

  return (
    <header className="chrome" data-chrome="1" data-state={state}>
      <div className="chrome-row">
        <span className="chrome-dot" data-state={state} />
        <span className="chrome-place" title="the place (worktree) these documents come from">{meta.place}</span>
        <span className="chrome-sep">·</span>
        <span className="chrome-branch">{meta.branch}</span>
        <span className="chrome-sep">·</span>
        <span className="chrome-behind" data-behind={meta.behind > 0 ? "1" : "0"}>{behind}</span>
        <span className="chrome-sep">·</span>
        <span className="chrome-dirty" data-dirty={meta.dirty > 0 ? "1" : "0"}>{dirty}</span>
      </div>
      <div className="chrome-row chrome-meta">
        <span className="chrome-subject" title={meta.subject}>
          {meta.subject ? `last commit: ${meta.subject}` : "no commit subject"}
        </span>
        <span className="chrome-times">
          <span className="chrome-time" title={`derived_epoch ${meta.derived_epoch}`}>
            text derived {ago(meta.derived_epoch, now)} ({clock(meta.derived_epoch)})
          </span>
          {split && (
            <span className="chrome-time chrome-time-split" title={`status_epoch ${meta.status_epoch}`}>
              git state measured {ago(meta.status_epoch, now)} ({clock(meta.status_epoch)})
            </span>
          )}
        </span>
      </div>
      <div className="chrome-row chrome-nav">{nav}</div>
    </header>
  );
}

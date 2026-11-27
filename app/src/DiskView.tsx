// The Disk view: what every project, workspace and place holds on disk, drawn
// as a cushion treemap (van Wijk & van de Wetering 1999, on the squarified
// layout in diskmodel.ts), with deleting from it. The engine counts
// (`worktrees_core::diskusage`); this file draws and asks.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { invoke } from "./ipc";
import { useEscape } from "./useEscape";
import * as Icons from "./icons";
import {
  CATS, CAT_LABEL, ago, bulkDefault, bulkRows, findPath, gb, isGone, ownTotal, placeTM, scannedAgo, scansAgo, squarify, staleOf, staleWords, tmValue, treeTM,
  type BulkRow, type Cat, type Cleanable, type Deleted, type DiskScope, type PlaceTree, type ProjectUsage, type Stale, type TM,
} from "./diskmodel";
import "./DiskView.css";

export type DiskProject = { root: string; name: string; places: { slug: string; path: string; last_commit_epoch?: number | null }[] };
export type DiskWorkspace = { id: string; name: string; projects: { root: string }[] };

// ── the engine's answers, held by App (the nav's badges read them too) ─────

type Job = { repo: string; slug: string; path: string; placePath: string; id?: string; label: string };

export type Disk = ReturnType<typeof useDisk>;

/** Scans run one at a time, in the order asked: a scan reads every file of a
 *  project, and two at once only fight over the disk. `later` holds a delete
 *  until its place's agent is idle (`busy` no longer has the place's path). */
export function useDisk(onError: (e: unknown) => void, busy: ReadonlySet<string>) {
  const [usage, setUsage] = useState<Record<string, ProjectUsage>>({});
  const [scanning, setScanning] = useState<{ root: string; left: number } | null>(null);
  const [waiting, setWaiting] = useState<Job[]>([]);
  const chain = useRef<Promise<void>>(Promise.resolve());
  const pending = useRef(0);
  const peeked = useRef(new Set<string>());

  const scan = useCallback((roots: string[], refresh: boolean) => {
    pending.current += roots.length;
    for (const root of roots) {
      chain.current = chain.current.then(async () => {
        setScanning({ root, left: pending.current });
        try {
          const u = await invoke<ProjectUsage | null>("disk_usage", { repo: root, refresh });
          if (u) setUsage((m) => ({ ...m, [root]: u }));
        } catch (e) {
          onError(e);
        } finally {
          pending.current -= 1;
          if (pending.current === 0) setScanning(null);
        }
      });
    }
    return chain.current;
  }, [onError]);

  /** What the engine already has, never a scan: for the nav's badges. */
  const peek = useCallback((roots: string[]) => {
    for (const root of roots) {
      if (peeked.current.has(root)) continue;
      peeked.current.add(root);
      invoke<ProjectUsage | null>("disk_usage", { repo: root, cached: true })
        .then((u) => { if (u) setUsage((m) => (m[root] ? m : { ...m, [root]: u })); })
        .catch(() => { /* an older engine without the row: no badges */ });
    }
  }, []);

  const del = useCallback(async (j: Job): Promise<Deleted> => {
    const d = await invoke<Deleted>("disk_delete", { repo: j.repo, slug: j.slug, path: j.path, scanned: j.id ?? null });
    return d;
  }, []);

  const later = useCallback((j: Job) => setWaiting((w) => [...w, j]), []);
  const cancelLater = useCallback((j: Job) => setWaiting((w) => w.filter((x) => x !== j)), []);

  // A held delete runs once its place's agent is idle, then the project rescans.
  useEffect(() => {
    const ready = waiting.filter((j) => !busy.has(j.placePath));
    if (!ready.length) return;
    setWaiting((w) => w.filter((j) => !ready.includes(j)));
    void (async () => {
      const roots = new Set<string>();
      for (const j of ready) {
        try { await del(j); roots.add(j.repo); } catch (e) { onError(`${j.label}: ${String(e)}`); }
      }
      if (roots.size) void scan([...roots], true);
    })();
  }, [busy, waiting, del, scan, onError]);

  return { usage, scanning, scan, peek, del, later, cancelLater, waiting };
}

// ── the treemap ────────────────────────────────────────────────────────────

type Rect = { t: TM; x: number; y: number; w: number; h: number; depth: number };

const PALETTE = {
  dark: { build: [0xd9, 0x59, 0x26], deps: [0x39, 0x87, 0xe5], source: [0x19, 0x9e, 0x70], git: [0x7a, 0x80, 0x9c] },
  light: { build: [0xc4, 0x50, 0x1f], deps: [0x2f, 0x74, 0xcc], source: [0x16, 0x87, 0x5f], git: [0x8a, 0x90, 0xaa] },
} as const;

function parseColor(s: string): [number, number, number] {
  const m = s.match(/[\d.]+/g);
  if (!m || m.length < 3) return [15, 15, 22];
  if (s.startsWith("color(")) return [Number(m[0]) * 255, Number(m[1]) * 255, Number(m[2]) * 255];
  return [Number(m[0]), Number(m[1]), Number(m[2])];
}

export function catColor(c: Cat, light: boolean): string {
  const [r, g, b] = (light ? PALETTE.light : PALETTE.dark)[c];
  return `rgb(${r},${g},${b})`;
}

function useLight(): boolean {
  const [light, setLight] = useState(false);
  useEffect(() => {
    const look = () => {
      const [r, g, b] = parseColor(getComputedStyle(document.body).backgroundColor);
      setLight((0.2126 * r + 0.7152 * g + 0.0722 * b) / 255 > 0.5);
    };
    look();
    const mo = new MutationObserver(look);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "style", "class"] });
    return () => mo.disconnect();
  }, []);
  return light;
}

export function Treemap({ root, selected, onPick, onOpen, light }: {
  root: TM;
  selected?: string | null;
  onPick?: (t: TM) => void;
  onOpen?: (t: TM) => void;
  light: boolean;
}) {
  const box = useRef<HTMLDivElement>(null);
  const cv = useRef<HTMLCanvasElement>(null);
  const rects = useRef<Rect[]>([]);
  const [size, setSize] = useState<{ w: number; h: number }>({ w: 0, h: 0 });
  const [tip, setTip] = useState<{ x: number; y: number; t: TM } | null>(null);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    setSize({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const c = cv.current;
    const { w: W, h: H } = size;
    if (!c || W < 10 || H < 10) return;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.round(W * dpr);
    c.height = Math.round(H * dpr);
    const g = c.getContext("2d");
    if (!g) return;
    const bg = parseColor(getComputedStyle(c).backgroundColor);
    g.fillStyle = `rgb(${bg.join(",")})`;
    g.fillRect(0, 0, c.width, c.height);
    const img = g.getImageData(0, 0, c.width, c.height);
    const px = img.data;
    const out: Rect[] = [];
    const labels: { t: TM; x: number; y: number; w: number; h: number; kind: TM["kind"] }[] = [];
    const pal = light ? PALETTE.light : PALETTE.dark;
    const hatch = light ? [[150, 156, 180], [190, 196, 214]] : [[68, 76, 114], [38, 43, 66]];
    // Cushion: each nesting level adds a parabolic ridge, lit from the top left.
    const ridge = (s: number[], x1: number, x2: number, y1: number, y2: number, hh: number) => {
      const r = s.slice();
      if (x2 > x1) { r[0] += (4 * hh * (x2 + x1)) / (x2 - x1); r[1] -= (4 * hh) / (x2 - x1); }
      if (y2 > y1) { r[2] += (4 * hh * (y2 + y1)) / (y2 - y1); r[3] -= (4 * hh) / (y2 - y1); }
      return r;
    };
    const Lx = -0.0976, Ly = -0.195, Lz = 0.976;
    const paint = (t: TM, x: number, y: number, w: number, h: number, s: number[]) => {
      const col = t.shared ? null : pal[t.cat ?? "git"];
      const X0 = Math.round(x * dpr), X1 = Math.round((x + w) * dpr) - (w > 4 ? 1 : 0);
      const Y0 = Math.round(y * dpr), Y1 = Math.round((y + h) * dpr) - (h > 4 ? 1 : 0);
      for (let py = Y0; py < Y1; py++) {
        for (let qx = X0; qx < X1; qx++) {
          const ux = (qx + 0.5) / dpr, uy = (py + 0.5) / dpr;
          const nx = -(2 * s[1] * ux + s[0]), ny = -(2 * s[3] * uy + s[2]);
          let k = (nx * Lx + ny * Ly + Lz) / Math.sqrt(nx * nx + ny * ny + 1);
          k = 0.46 + 0.62 * Math.max(0, k);
          const cc = col ?? hatch[(qx + py) % 14 < 7 ? 0 : 1];
          const i = (py * c.width + qx) * 4;
          px[i] = Math.min(255, cc[0] * k); px[i + 1] = Math.min(255, cc[1] * k); px[i + 2] = Math.min(255, cc[2] * k); px[i + 3] = 255;
        }
      }
    };
    const rec = (t: TM, x: number, y: number, w: number, h: number, s: number[], hh: number, depth: number) => {
      if (w < 1 || h < 1) return;
      if (t.kind === "group") {
        x += 2; y += 2; w -= 4; h -= 4;
        if (w < 6 || h < 6) return;
        out.push({ t, x, y, w, h, depth });
        if (w > 70 && h > 44) { labels.push({ t, x, y, w, h, kind: "group" }); y += 19; h -= 19; }
      } else {
        out.push({ t, x, y, w, h, depth });
        if (t.kind === "place" && w > 74 && h > 30) labels.push({ t, x: x + 5, y: y + 5, w: w - 10, h, kind: "place" });
        if (t.kind === "dir" && depth > 0 && depth <= 3 && w > 90 && h > 30 && t.label) labels.push({ t, x: x + 6, y: y + 6, w: w - 12, h, kind: "dir" });
        if (depth > 0) s = ridge(s, x, x + w, y, y + h, hh);
      }
      if (t.kids) {
        const kids = t.kids.slice().sort((a, b) => tmValue(b) - tmValue(a));
        for (const [i, kx, ky, kw, kh] of squarify(kids.map(tmValue), x, y, w, h)) {
          rec(kids[i], kx, ky, kw, kh, s, t.kind === "group" || depth === 0 ? hh : hh * 0.78, depth + 1);
        }
        return;
      }
      paint(t, x, y, w, h, s);
    };
    rec(root, 0, 0, W, H, [0, 0, 0, 0], 0.5, 0);
    g.putImageData(img, 0, 0);
    g.scale(dpr, dpr);
    g.textBaseline = "top";
    // Outlines: a workspace's or (main)'s places' dashed box, and the selection.
    for (const r of out) {
      if (r.t.box) {
        g.strokeStyle = light ? "#3b4261" : "#c0caf5"; g.setLineDash([5, 4]); g.lineWidth = 1.5;
        g.strokeRect(r.x + 1, r.y + 1, r.w - 2, r.h - 2); g.setLineDash([]);
      }
      if (selected && r.t.key === selected) {
        g.strokeStyle = "#fff"; g.lineWidth = 3; g.strokeRect(r.x + 1.5, r.y + 1.5, r.w - 3, r.h - 3);
        g.strokeStyle = "#000"; g.lineWidth = 1; g.strokeRect(r.x + 4, r.y + 4, r.w - 8, r.h - 8);
      }
    }
    const font = "-apple-system, system-ui, sans-serif";
    // Labels: dark plates whatever the theme, so white text reads on any hue.
    let dirOffset = new Map<string, number>();
    for (const l of labels) {
      const v = gb(tmValue(l.t));
      if (l.kind === "group") {
        g.font = `600 12.5px ${font}`;
        g.fillStyle = light ? "#1a1b26" : "#e6eaff";
        const full = `${l.t.label}  ${v}`;
        g.fillText(fit(g, g.measureText(full).width < l.w - 8 ? full : l.t.label, l.w - 8), l.x + 3, l.y + 3);
        continue;
      }
      g.font = `500 11.5px ${font}`;
      const off = dirOffset.get(l.t.key) ?? 0;
      const full = `${l.t.label}  ${v}`;
      const two = l.kind === "place" && g.measureText(full).width > l.w - 12 && l.h > 50;
      const lines = two ? [fit(g, l.t.label, l.w - 12), v] : [g.measureText(full).width <= l.w - 12 ? full : fit(g, l.t.label, l.w - 12)];
      const bw = Math.max(...lines.map((s) => g.measureText(s).width)) + 10;
      g.fillStyle = "rgba(15,15,22,.68)";
      g.beginPath(); g.roundRect(l.x, l.y + off, bw, lines.length * 14 + 5, 4); g.fill();
      lines.forEach((s, i) => { g.fillStyle = i ? "#c0caf5" : "#fff"; g.fillText(s, l.x + 5, l.y + off + 4 + i * 14); });
      // A folder's biggest child labels below its parent's plate.
      const kid = l.t.kids?.slice().sort((a, b) => tmValue(b) - tmValue(a))[0];
      if (kid) dirOffset.set(kid.key, off + 20);
    }
    dirOffset = new Map();
    rects.current = out;
  }, [root, size, selected, light]);

  const hit = (e: React.MouseEvent): Rect | null => {
    const b = box.current!.getBoundingClientRect();
    const x = e.clientX - b.left, y = e.clientY - b.top;
    let best: Rect | null = null;
    for (const r of rects.current) if (x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h && (!best || r.depth >= best.depth)) best = r;
    return best;
  };
  /** The deepest rectangle a click can act on: one with a path or a place. */
  const actionable = (r: Rect | null): TM | null => {
    if (!r) return null;
    const chain = rects.current.filter((q) => q.x <= r.x + 0.5 && q.y <= r.y + 0.5 && q.x + q.w >= r.x + r.w - 0.5 && q.y + q.h >= r.y + r.h - 0.5).sort((a, b) => b.depth - a.depth);
    return chain.find((q) => q.t.ref?.path || q.t.ref?.slug || q.t.ref?.root)?.t ?? null;
  };
  return (
    <div className="dk-map" ref={box}
      onMouseMove={(e) => { const r = hit(e); const b = box.current!.getBoundingClientRect(); setTip(r ? { x: e.clientX - b.left, y: e.clientY - b.top, t: actionable(r) ?? r.t } : null); }}
      onMouseLeave={() => setTip(null)}
      onClick={(e) => { const t = actionable(hit(e)); if (t) onPick?.(t); }}
      onDoubleClick={(e) => { const t = actionable(hit(e)); if (t) onOpen?.(t); }}
      data-testid="disk-map">
      <canvas ref={cv} role="img" aria-label="Treemap of disk usage; the list beside it has the same numbers" />
      {tip && (
        <div className="dk-tip" style={{ left: Math.min(tip.x + 14, size.w - 230), top: Math.min(tip.y + 14, size.h - 60) }}>
          <b>{tip.t.ref?.path ?? (tip.t.label || (tip.t.cat ? CAT_LABEL[tip.t.cat] : ""))}</b>
          <span>{gb(tmValue(tip.t))}{tip.t.gone ? " · removed after this scan" : tip.t.shared ? " · shared with other places" : tip.t.cat && tip.t.kind === "leaf" && !tip.t.label ? ` · small ${CAT_LABEL[tip.t.cat].toLowerCase()}` : ""}</span>
        </div>
      )}
    </div>
  );
}

function fit(g: CanvasRenderingContext2D, s: string, w: number): string {
  if (g.measureText(s).width <= w) return s;
  while (s.length > 2 && g.measureText(s + "…").width > w) s = s.slice(0, -1);
  return s + "…";
}

function Legend({ shared, extra, light }: { shared?: boolean; extra?: React.ReactNode; light: boolean }) {
  return (
    <div className="dk-leg">
      {CATS.map((c) => <span key={c}><i style={{ background: catColor(c, light) }} />{CAT_LABEL[c]}</span>)}
      {shared && <span><i className="dk-hatch" />Shared with other places: removing this one does not free it</span>}
      {extra}
    </div>
  );
}

function Bar({ own, max, shared, light }: { own: Record<Cat, number>; max: number; shared?: number; light: boolean }) {
  if (max <= 0) return null;
  return (
    <div className="dk-bar">
      {CATS.map((c) => own[c] > max / 400 ? <s key={c} style={{ background: catColor(c, light), width: `${(own[c] / max) * 100}%` }} /> : null)}
      {shared ? <s className="dk-hatch" style={{ width: `${(shared / max) * 100}%` }} /> : null}
    </div>
  );
}

const Tile = ({ label, value, note }: { label: React.ReactNode; value: string; note: React.ReactNode }) => (
  <div className="dk-t"><div className="l">{label}</div><div className="v">{value}</div><div className="n">{note}</div></div>
);

/** What the kept scan no longer matches, in one line under the header, with
 *  the one way to fix it. Never a scan of its own: Rescan is the user's. */
function StaleNotice({ words, onRescan, scanning }: { words: string[]; onRescan: () => void; scanning: boolean }) {
  if (!words.length) return null;
  return (
    <div className="dk-stale" role="status" data-testid="disk-stale">
      <i className="dk-stale-dot" />
      <span>{words.join(" ")}</span>
      <button className="ctrl sm" onClick={onRescan} disabled={scanning} data-testid="disk-stale-rescan">{scanning ? "Scanning…" : "Rescan"}</button>
    </div>
  );
}

/** The age a minute-old screen should say: ticks once a minute, reads no file. */
function useNow(): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const t = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 60_000);
    return () => window.clearInterval(t);
  }, []);
  return now;
}

// ── the view ───────────────────────────────────────────────────────────────

export function DiskView({ scope, path, pick, navButtons, projects, workspaces, byWorkspace, busy, disk, open, close, onError }: {
  scope: DiskScope;
  /** What is picked in a place's treemap, and the way to pick: App holds it,
   *  because it is part of the history location (Back unpicks). */
  path: string | null;
  pick: (path: string | null) => void;
  /** The app's ‹ ›, when no topbar above carries them (Home, a workspace view). */
  navButtons: React.ReactNode;
  projects: DiskProject[];
  workspaces: DiskWorkspace[] | null;
  byWorkspace: boolean;
  busy: ReadonlySet<string>;
  disk: Disk;
  open: (s: DiskScope) => void;
  close: () => void;
  onError: (e: unknown) => void;
}) {
  const light = useLight();
  const now = useNow();
  // Escape closes Disk. Not `useEscape`: that stack is for modal surfaces and
  // takes the key from everything, a dock terminal included, and Disk is not
  // modal. A bubble listener hears Escape only where nothing else used it:
  // xterm stops the ones it handles, and a dialog above (Delete…, Clean up…)
  // takes it in useEscape's capture phase first.
  const closeRef = useRef(close);
  closeRef.current = close;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || e.repeat || e.isComposing) return;
      const t = e.target;
      if (t instanceof HTMLElement && (t.isContentEditable || t.closest("input, textarea, select, .term-host, .cm-editor"))) return;
      // A menu, or the sidebar shown over the panes (App closes that one, and
      // one Escape should close one thing).
      if (document.querySelector(".menu-catch, .nav.overlay")) return;
      closeRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const [withShared, setWithShared] = useState(scope.kind === "place");
  const [bulk, setBulk] = useState(false);
  const names = useMemo(() => Object.fromEntries(projects.map((p) => [p.root, p.name])), [projects]);
  const placeOf = useCallback((repo: string, slug: string) => projects.find((p) => p.root === repo)?.places.find((x) => x.slug === slug), [projects]);
  const info = useCallback((repo: string, slug: string) => {
    const p = placeOf(repo, slug);
    return { last: p?.last_commit_epoch ?? null, busy: !!p && busy.has(p.path) };
  }, [placeOf, busy]);

  // Which projects this scope reads.
  const roots = useMemo(() => {
    if (scope.kind === "project") return [scope.root];
    if (scope.kind === "place") return [scope.repo];
    if (scope.kind === "workspace") return (workspaces?.find((w) => w.id === scope.id)?.projects ?? []).map((p) => p.root).filter((r) => names[r]);
    return projects.map((p) => p.root);
  }, [scope, workspaces, projects, names]);

  // What the kept scans no longer match: compared with the places the app
  // already knows, never by scanning (diskmodel's staleness rules).
  const live = useCallback((r: string) => projects.find((p) => p.root === r)?.places ?? null, [projects]);
  const stale = useMemo(() => staleOf(roots, disk.usage, live), [roots, disk.usage, live]);
  const goneHere = useCallback((repo: string, slug: string) => isGone(stale, repo, slug), [stale]);

  // Scan what the engine has not yet: once per view. A project with a kept
  // scan is never scanned again here, however old: Rescan is the only way.
  const asked = useRef(false);
  useEffect(() => {
    if (asked.current || scope.kind === "place") return;
    asked.current = true;
    const missing = roots.filter((r) => !disk.usage[r]);
    if (missing.length) void disk.scan(missing, false);
  }, [roots, disk, scope.kind]);

  const header = (title: React.ReactNode, sub: React.ReactNode, onRescan: () => void, scanning: boolean, extra?: React.ReactNode) => (
    <div className="dk-h">
      {navButtons}
      <div className="dk-title"><h2>{title}</h2><div className="sub">{sub}</div></div>
      <span className="sp" />
      {extra}
      <div className="dk-seg" role="group" aria-label="What to count">
        <button className={withShared ? "" : "on"} onClick={() => setWithShared(false)} title="Bytes only this place holds: what removing it frees">Only here</button>
        <button className={withShared ? "on" : ""} onClick={() => setWithShared(true)} title="Also the bytes it shares with other places (clones, hard links)">On disk</button>
      </div>
      <button className="ctrl sm" onClick={onRescan} disabled={scanning} data-testid="disk-rescan">{scanning ? "Scanning…" : "Rescan"}</button>
      <button className="ctrl sm icon" onClick={close} title="Close Disk" aria-label="Close Disk" data-testid="disk-close"><Icons.X size={13} /></button>
    </div>
  );

  if (scope.kind === "place") {
    // A place the app knows was removed: nothing to walk, and nothing to delete.
    const removed = !!live(scope.repo) && !placeOf(scope.repo, scope.slug);
    return <PlaceDisk scope={scope} name={names[scope.repo] ?? scope.repo} info={info(scope.repo, scope.slug)} placePath={placeOf(scope.repo, scope.slug)?.path ?? ""}
      header={header} withShared={withShared} disk={disk} onError={onError} light={light} now={now} open={open} infoOf={info} path={path} pick={pick}
      removed={removed} stale={stale} gone={goneHere} />;
  }

  const have = roots.filter((r) => disk.usage[r]);
  const scanning = !!disk.scanning;
  const scanNote = disk.scanning ? `scanning ${names[disk.scanning.root] ?? disk.scanning.root}… ${disk.scanning.left} left` : null;
  const allPlaces = have.flatMap((r) => disk.usage[r].places.map((p) => ({ repo: r, p })));
  const sumOwn = allPlaces.reduce((a, x) => a + ownTotal(x.p.own), 0);
  const unique = have.reduce((a, r) => a + disk.usage[r].unique, 0);
  const du = allPlaces.reduce((a, x) => a + x.p.alloc, 0);
  const build = allPlaces.reduce((a, x) => a + x.p.own.build, 0);
  const vol = have.length ? disk.usage[have[0]].volume : null;
  const snaps = vol?.snapshots ?? 0;
  const rescan = () => void disk.scan(roots, true);
  // A project still in the queue is not "not scanned": it is being scanned.
  const words = staleWords(scanning ? { ...stale, unscanned: [] } : stale, (r) => names[r] ?? r);
  const notice = <StaleNotice words={words} onRescan={rescan} scanning={scanning} />;
  const age = scansAgo(now, have.map((r) => disk.usage[r]));
  const projTM = (r: string): TM => ({
    key: r, label: names[r] ?? r, value: 0, kind: "group", ref: { root: r },
    kids: disk.usage[r].places.map((p) => placeTM(r, p, withShared, goneHere(r, p.slug))).filter((t) => tmValue(t) > 0),
  });
  const pickPlace = (t: TM) => {
    if (t.gone) return;
    if (t.ref?.slug && t.ref.repo) open({ kind: "place", repo: t.ref.repo, slug: t.ref.slug });
    else if (t.ref?.root) open({ kind: "project", root: t.ref.root });
  };
  const bulkBtn = <button className="ctrl sm" onClick={() => setBulk(true)} disabled={!have.length} data-testid="disk-bulk">Clean up…</button>;
  const snapNote = snaps > 0 ? <div className="dk-note">{snaps} Time Machine local snapshot{snaps > 1 ? "s" : ""} on this Mac: deleted files can keep their space until the snapshots expire, about a day.</div> : null;
  const bulkSheet = bulk && (
    <BulkSheet rows={bulkRows(Object.fromEntries(have.map((r) => [r, disk.usage[r]])), names, info, undefined, goneHere)} now={now} disk={disk}
      left={stale.gone.length} onClose={() => setBulk(false)} onError={onError} />
  );

  if (scope.kind === "project") {
    const u = disk.usage[scope.root];
    const name = names[scope.root] ?? scope.root;
    const rows = u ? u.places.slice().sort((a, b) => ownTotal(b.own) + (withShared ? b.shared : 0) - ownTotal(a.own) - (withShared ? a.shared : 0)) : [];
    const mx = rows.length ? Math.max(...rows.map((p) => ownTotal(p.own) + p.shared)) : 0;
    const main = u?.places.find((p) => p.is_main);
    return (
      <div className="dk" data-testid="disk-view">
        {header(name, u ? `Project · ${u.places.length} places · scanned ${scannedAgo(now, u.scanned_epoch)} · took ${(u.scan_ms / 1000).toFixed(1)} s` : scanNote ?? "not scanned yet", rescan, scanning, bulkBtn)}
        {notice}
        {u ? (
          <>
            <div className="dk-tiles">
              <Tile label="Counted once" value={gb(u.unique)} note={`du says ${gb(u.places.reduce((a, p) => a + p.alloc, 0))}: it counts every clone`} />
              <Tile label="Held by one place only" value={gb(sumOwn)} note={main ? `(main) ${gb(ownTotal(main.own))}` : ""} />
              <Tile label="Build output" value={gb(build)} note="rebuildable" />
              <Tile label="Free on this volume" value={vol ? gb(vol.free) : "—"} note={vol ? `of ${gb(vol.total)}` : ""} />
            </div>
            <div className="dk-body">
              <Treemap root={{ key: "/", label: "", value: 0, kind: "dir", kids: [projTM(scope.root)] }} onPick={pickPlace} light={light} />
              <div className="dk-side">
                <div className="cap">PLACES<i />{withShared ? "ON DISK" : "ONLY HERE"}</div>
                <div className="dk-list">
                  {rows.map((p) => {
                    const pi = info(scope.root, p.slug);
                    const g = goneHere(scope.root, p.slug);
                    return (
                      <button className={g ? "dk-r dk-gone" : "dk-r"} key={p.slug} disabled={g} data-gone={g || undefined} data-slug={p.slug} data-track="disk-place-row"
                        title={g ? `${p.slug} was removed after this scan` : undefined}
                        onClick={() => open({ kind: "place", repo: scope.root, slug: p.slug })}>
                        <b>{g && <i className="dk-stale-dot" />}{p.slug}{pi.busy && <span className="dk-live"> ● agent</span>}</b>
                        <span className="num">{gb(ownTotal(p.own) + (withShared ? p.shared : 0))}</span>
                        <small>{g ? "removed after this scan" : `last commit ${ago(now, pi.last)}`}</small>
                        <small>{p.shared > 1e8 ? `shares ${gb(p.shared)}` : ""}</small>
                        <Bar own={p.own} max={mx} shared={withShared ? p.shared : 0} light={light} />
                      </button>
                    );
                  })}
                </div>
                {snapNote}
              </div>
            </div>
            <Legend shared={withShared} light={light} />
          </>
        ) : <div className="dk-empty">{scanNote ?? "Scanning…"}</div>}
        {bulkSheet}
      </div>
    );
  }

  // Home ("all") and a workspace: groups of projects.
  const ws = scope.kind === "workspace" ? workspaces?.find((w) => w.id === scope.id) : null;
  type Group = { key: string; name: string; roots: string[]; derived?: boolean };
  let groups: Group[];
  if (scope.kind === "workspace") {
    groups = [{ key: scope.id, name: ws?.name ?? "Workspace", roots }];
  } else if (byWorkspace && workspaces) {
    const inAny = new Set(workspaces.flatMap((w) => w.projects.map((p) => p.root)));
    groups = [
      ...workspaces.map((w) => ({ key: w.id, name: w.name, roots: w.projects.map((p) => p.root).filter((r) => names[r]) })),
      { key: "else", name: "Everything Else", roots: projects.map((p) => p.root).filter((r) => !inAny.has(r)), derived: true },
    ];
  } else {
    groups = [];
  }
  const ownOf = (r: string) => disk.usage[r]?.places.reduce((a, p) => a + ownTotal(p.own) + (withShared ? p.shared : 0), 0) ?? 0;
  // The map draws a project once: under the first group that holds it.
  const drawn = new Set<string>();
  const mapRoot: TM = groups.length
    ? {
      key: "/", label: "", value: 0, kind: "dir",
      kids: groups.map((gr) => ({
        key: "g|" + gr.key, label: gr.name, value: 0, kind: "group" as const, box: true,
        kids: gr.roots.filter((r) => disk.usage[r] && !drawn.has(r) && (drawn.add(r), true)).map((r) => projTM(r)),
      })),
    }
    : { key: "/", label: "", value: 0, kind: "dir", kids: have.map(projTM) };
  const largest = allPlaces.filter((x) => !x.p.is_main || scope.kind === "workspace").sort((a, b) => ownTotal(b.p.own) - ownTotal(a.p.own)).slice(0, 12);
  const mx = largest.length ? ownTotal(largest[0].p.own) : 0;
  const title = scope.kind === "workspace" ? ws?.name ?? "Workspace" : "Disk";
  const sub = `${scope.kind === "workspace" ? "Workspace · " : groups.length ? "By workspace · " : "All projects · "}${have.length} of ${roots.length} projects scanned · ${allPlaces.length} places${age ? " · " + age : ""}${scanNote ? " · " + scanNote : ""}`;
  return (
    <div className="dk" data-testid="disk-view">
      {header(title, sub, rescan, scanning, bulkBtn)}
      {notice}
      <div className="dk-tiles">
        {groups.length > 1
          ? groups.map((gr) => (
            <Tile key={gr.key} label={<>{gr.name}{gr.derived && <span className="dk-tag">derived</span>}</>}
              value={gr.roots.length ? gb(gr.roots.reduce((a, r) => a + ownOf(r), 0)) : "—"}
              note={gr.roots.length ? `${gr.roots.length} project${gr.roots.length > 1 ? "s" : ""}` : "no projects yet"} />
          ))
          : <>
            <Tile label="Counted once" value={gb(unique)} note={`du says ${gb(du)}: it counts every clone`} />
            <Tile label="Held by one place only" value={gb(sumOwn)} note="what removing places can free" />
            <Tile label="Build output" value={gb(build)} note="rebuildable, every place and main" />
          </>}
        <Tile label="Free on this volume" value={vol ? gb(vol.free) : "—"} note={vol ? `of ${gb(vol.total)}` : ""} />
      </div>
      <div className="dk-body">
        {have.length ? <Treemap root={mapRoot} onPick={pickPlace} light={light} /> : <div className="dk-map dk-empty">{scanNote ?? "Nothing scanned yet."}</div>}
        <div className="dk-side">
          {groups.length > 1 ? (
            <>
              <div className="cap">WORKSPACE › PROJECT<i />{withShared ? "ON DISK" : "ONLY HERE"}</div>
              <div className="dk-list">
                {groups.filter((gr) => gr.roots.length).map((gr) => (
                  <div key={gr.key}>
                    <div className="dk-wsh"><b>{gr.name}</b><span className="num">{gb(gr.roots.reduce((a, r) => a + ownOf(r), 0))}</span></div>
                    {gr.roots.slice().sort((a, b) => ownOf(b) - ownOf(a)).map((r) => (
                      <button key={r} className="dk-r dk-sub" onClick={() => open({ kind: "project", root: r })}>
                        <b>{names[r]}</b><span className="num">{disk.usage[r] ? gb(ownOf(r)) : "…"}</span>
                      </button>
                    ))}
                  </div>
                ))}
              </div>
              <div className="dk-note">A project in two workspaces counts in each, and once on the map.</div>
            </>
          ) : (
            <>
              <div className="cap">LARGEST PLACES<i />ONLY HERE</div>
              <div className="dk-list">
                {largest.map(({ repo, p }) => {
                  const pi = info(repo, p.slug);
                  const g = goneHere(repo, p.slug);
                  return (
                    <button className={g ? "dk-r dk-gone" : "dk-r"} key={repo + p.slug} disabled={g} data-gone={g || undefined} data-slug={p.slug} data-track="disk-place-row"
                      title={g ? `${p.slug} was removed after this scan` : undefined}
                      onClick={() => open({ kind: "place", repo, slug: p.slug })}>
                      <b>{g && <i className="dk-stale-dot" />}{p.slug}{pi.busy && <span className="dk-live"> ● agent</span>}</b>
                      <span className="num">{gb(ownTotal(p.own))}</span>
                      <small>{names[repo]}</small>
                      <small>{g ? "removed after this scan" : `last commit ${ago(now, pi.last)}`}</small>
                      <Bar own={p.own} max={mx} light={light} />
                    </button>
                  );
                })}
              </div>
            </>
          )}
          {snapNote}
        </div>
      </div>
      <Legend shared={withShared} light={light} extra={groups.length ? <span><i className="dk-dash" />A workspace</span> : null} />
      {bulkSheet}
    </div>
  );
}

// ── one place ──────────────────────────────────────────────────────────────

function PlaceDisk({ scope, name, info, placePath, header, withShared, disk, onError, light, now, open, infoOf, path, pick, removed, stale, gone }: {
  /** The app's snapshot no longer has this place: nothing is walked or deleted. */
  removed: boolean;
  stale: Stale;
  gone: (repo: string, slug: string) => boolean;
  path: string | null;
  pick: (path: string | null) => void;
  scope: { kind: "place"; repo: string; slug: string };
  name: string;
  info: { last: number | null; busy: boolean };
  placePath: string;
  header: (title: React.ReactNode, sub: React.ReactNode, onRescan: () => void, scanning: boolean, extra?: React.ReactNode) => React.ReactNode;
  withShared: boolean;
  disk: Disk;
  onError: (e: unknown) => void;
  light: boolean;
  now: number;
  open: (s: DiskScope) => void;
  infoOf: (repo: string, slug: string) => { last: number | null; busy: boolean };
}) {
  const [tree, setTree] = useState<PlaceTree | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<{ path: string; id?: string; bytes: number; files: number; cat?: Cat } | null>(null);
  const load = useCallback(() => {
    if (removed) return;
    setLoading(true);
    setErr(null);
    invoke<PlaceTree>("disk_place_tree", { repo: scope.repo, slug: scope.slug })
      .then((t) => setTree(t))
      .catch((e) => setErr(String(e)))
      .finally(() => setLoading(false));
  }, [scope.repo, scope.slug, removed]);
  useEffect(load, [load]);
  // (main)'s folder holds every other place: they are drawn beside its own
  // tree in a dashed box, each its own place, never part of (main)'s numbers.
  const isMain = scope.slug === "(main)";
  const others = isMain ? (disk.usage[scope.repo]?.places ?? []).filter((p) => !p.is_main) : [];
  const askedMain = useRef(false);
  useEffect(() => {
    if (!isMain || removed || askedMain.current || disk.usage[scope.repo]) return;
    askedMain.current = true;
    void disk.scan([scope.repo], false);
  }, [isMain, scope.repo, disk]);
  const tm = useMemo(() => {
    if (!tree) return null;
    const own = treeTM(tree.tree, withShared);
    const kids = others.map((p) => placeTM(scope.repo, p, withShared, gone(scope.repo, p.slug))).filter((t) => tmValue(t) > 0);
    if (!kids.length) return own;
    return {
      key: "/", label: "", value: 0, kind: "dir" as const,
      kids: [
        { ...own, key: "/main", label: "this checkout", kind: "group" as const },
        { key: "/wt", label: ".worktrees/", value: 0, kind: "group" as const, box: true, kids },
      ],
    };
  }, [tree, withShared, others, scope.repo, gone]);
  const u = tree?.usage;
  const snaps = tree?.volume.snapshots ?? 0;
  const sel = tm && path ? findPath(tm, path) : null;
  const selNode = sel?.ref?.path ? sel : null;
  const sizeOf = (t: TM) => tmValue(t);
  const askDelete = (path: string, id: string | undefined, bytes: number, files: number, cat?: Cat) => setConfirm({ path, id, bytes, files, cat });
  const waitingHere = disk.waiting.filter((j) => j.repo === scope.repo && j.slug === scope.slug);
  // (main)'s box of other places comes from the project's kept scan: say what
  // it no longer matches. Its own tree is walked fresh each time it opens.
  const mainWords = isMain ? staleWords({ gone: stale.gone, added: stale.added, unscanned: [] }, (r) => r) : [];
  const rescanMain = () => { load(); void disk.scan([scope.repo], true); };
  if (removed) {
    return (
      <div className="dk" data-testid="disk-view">
        {header("Disk", <span className="dk-crumb">{name} › <b>{scope.slug}</b></span>, () => {}, true)}
        <div className="dk-empty" data-testid="disk-removed">{scope.slug} was removed. There is nothing left of it to count or delete.</div>
      </div>
    );
  }
  return (
    <div className="dk" data-testid="disk-view">
      {header("Disk",
        <span className="dk-crumb">{name} › <b>{scope.slug}</b> · last commit {ago(now, info.last)}{tree ? ` · scanned ${scannedAgo(now, tree.scanned_epoch)} · took ${(tree.scan_ms / 1000).toFixed(1)} s` : ""}</span>,
        isMain ? rescanMain : load, loading || (isMain && !!disk.scanning))}
      <StaleNotice words={mainWords} onRescan={rescanMain} scanning={loading || !!disk.scanning} />
      {err && <div className="dk-err">{err}</div>}
      {u && tm ? (
        <>
          <div className="dk-tiles">
            <Tile label="On disk" value={gb(ownTotal(u.own) + u.shared)} note={`du says ${gb(u.alloc)}`} />
            <Tile label={scope.slug === "(main)" ? "Only this checkout" : "Frees if removed"} value={gb(ownTotal(u.own))} note={scope.slug === "(main)" ? "(main) is the repository; it is never removed" : "only this place holds it"} />
            <Tile label="Shared" value={gb(u.shared)} note="with other places or a store" />
            <Tile label="Build output" value={gb(u.own.build)} note="rebuildable" />
          </div>
          <div className="dk-body">
            <Treemap root={tm} selected={sel?.key ?? null} light={light}
              onPick={(t) => { if (t.gone) return; if (t.ref?.slug && t.ref.repo) open({ kind: "place", repo: t.ref.repo, slug: t.ref.slug }); else pick(t.ref?.path ?? null); }} />
            <div className="dk-side">
              <div className="cap">SELECTED</div>
              {selNode ? (
                <div className="dk-selbox" data-testid="disk-selected">
                  <div className="dk-selp">{selNode.ref!.path}</div>
                  <div className="dk-selv">{gb(sizeOf(selNode))}</div>
                  <div className="sub">{selNode.cat ? CAT_LABEL[selNode.cat] : ""}{selNode.shared ? " · shared with other places" : ""}</div>
                  <div className="dk-acts">
                    <button className="ctrl sm danger" data-testid="disk-delete"
                      disabled={selNode.cat === "git"}
                      title={selNode.cat === "git" ? "git's own files are never deleted here" : "Only files git ignores can be deleted; it is checked before anything goes"}
                      onClick={() => askDelete(selNode.ref!.path!, selNode.ref!.id, sizeOf(selNode), 0, selNode.cat)}>Delete…</button>
                  </div>
                </div>
              ) : <div className="dk-hint">Click a block to select its file or folder.</div>}
              <div className="cap" style={{ marginTop: 14 }}>BUILD OUTPUT AND DEPENDENCIES<i />WOULD FREE</div>
              <div className="dk-list">
                {u.cleanable.map((c) => (
                  <div className="dk-r" key={c.path}>
                    <b className="mono">{c.path}</b><span className="num">{gb(c.own)}</span>
                    <small>{CAT_LABEL[c.cat]}{c.shared > 1e7 ? ` · ${gb(c.shared)} shared, stays` : ""}</small>
                    <button className="ctrl sm" onClick={() => askDelete(c.path, c.id, c.own, c.files, c.cat)}>Delete…</button>
                  </div>
                ))}
                {!u.cleanable.length && <div className="dk-hint">No build output or dependency folders.</div>}
              </div>
              {others.length > 0 && (
                <>
                  <div className="cap" style={{ marginTop: 14 }}>IN .worktrees/<i />ONLY THAT PLACE</div>
                  <div className="dk-list">
                    {others.slice().sort((a, b) => ownTotal(b.own) - ownTotal(a.own)).map((p) => {
                      const pi = infoOf(scope.repo, p.slug);
                      const g = gone(scope.repo, p.slug);
                      return (
                        <button className={g ? "dk-r dk-gone" : "dk-r"} key={p.slug} disabled={g} data-gone={g || undefined} data-slug={p.slug} data-track="disk-place-row"
                          title={g ? `${p.slug} was removed after this scan` : undefined}
                          onClick={() => open({ kind: "place", repo: scope.repo, slug: p.slug })}>
                          <b>{g && <i className="dk-stale-dot" />}{p.slug}{pi.busy && <span className="dk-live"> ● agent</span>}</b><span className="num">{gb(ownTotal(p.own))}</span>
                          <small>{g ? "removed after this scan" : `last commit ${ago(now, pi.last)}`}</small><small>{p.own.build > 5e8 ? `build ${gb(p.own.build)}` : ""}</small>
                        </button>
                      );
                    })}
                  </div>
                </>
              )}
              {waitingHere.map((j) => (
                <div className="dk-wait" key={j.path}>Deletes {j.path} when the agent is idle <button className="ctrl sm" onClick={() => disk.cancelLater(j)}>Cancel</button></div>
              ))}
              <div className="dk-note">
                Only files git ignores are deleted here, and permanently: the Trash would free nothing.
                {snaps > 0 && ` ${snaps} Time Machine local snapshot${snaps > 1 ? "s" : ""} can hold the space for about a day.`}
              </div>
            </div>
          </div>
          <Legend shared={withShared} light={light} extra={<span><i className="dk-selk" />Selected</span>} />
        </>
      ) : !err && <div className="dk-empty">Scanning {scope.slug}…</div>}
      {confirm && (
        <DeleteDialog target={confirm} slug={scope.slug} name={name} busy={info.busy} snaps={snaps}
          onClose={() => setConfirm(null)}
          now={async () => {
            await disk.del({ repo: scope.repo, slug: scope.slug, path: confirm.path, id: confirm.id, placePath, label: `${scope.slug}/${confirm.path}` });
            setConfirm(null);
            if (confirm.path === path) pick(null);
            load();
            void disk.scan([scope.repo], true);
          }}
          later={() => { disk.later({ repo: scope.repo, slug: scope.slug, path: confirm.path, id: confirm.id, placePath, label: `${scope.slug}/${confirm.path}` }); setConfirm(null); }}
          onError={onError} />
      )}
    </div>
  );
}

function DeleteDialog({ target, slug, name, busy, snaps, onClose, now, later, onError }: {
  target: { path: string; bytes: number; files: number; cat?: Cat };
  slug: string; name: string; busy: boolean; snaps: number;
  onClose: () => void; now: () => Promise<void>; later: () => void; onError: (e: unknown) => void;
}) {
  useEscape(onClose);
  const [working, setWorking] = useState(false);
  const [refused, setRefused] = useState<string | null>(null);
  const go = () => {
    setWorking(true);
    now().catch((e) => { setRefused(String(e)); setWorking(false); }).catch(onError);
  };
  const rebuild = target.cat === "deps" ? "A package install brings dependencies back." : target.cat === "build" ? "The next build brings it back." : "";
  return (
    <div className="scrim scrim-center" onClick={onClose}>
      <div className="sync-modal rm-modal dk-modal" role="dialog" aria-modal="true" aria-label={`Delete ${target.path}`} onClick={(e) => e.stopPropagation()}>
        <header className="sync-h"><b>Delete {target.path}?</b><span className="sync-hub">{name} › {slug}</span></header>
        <div className="sync-body">
          <div className="dk-free"><span>Frees about</span><b>{gb(target.bytes)}</b></div>
          <ul className="dk-bullets">
            <li>Deleted permanently, not moved to the Trash: the Trash would free nothing.</li>
            <li>Only if git ignores it and tracks nothing inside it; worktrees checks before anything goes.</li>
            {rebuild && <li>{rebuild}</li>}
          </ul>
          {busy && <div className="sync-live"><b>An agent is working in this place.</b> Deleting under a build that is running can make it fail. Delete when idle waits until the agent stops.</div>}
          {snaps > 0 && <div className="dk-hint">Time Machine's local snapshots can hold the space for about a day.</div>}
          {refused && <div className="dk-err" data-testid="disk-refused">{refused}</div>}
        </div>
        <footer className="sync-foot">
          <button className="ctrl" onClick={onClose}>Cancel</button>
          {busy && <button className="ctrl" onClick={go} disabled={working}>Delete now</button>}
          <button className="enter-btn danger" autoFocus data-testid="disk-delete-go" disabled={working} onClick={busy ? later : go}>
            {working ? "Deleting…" : busy ? "Delete when idle" : `Delete ${gb(target.bytes)}`}
          </button>
        </footer>
      </div>
    </div>
  );
}

function BulkSheet({ rows, now, disk, left, onClose, onError }: {
  rows: BulkRow[]; now: number; disk: Disk;
  /** Places the scan holds that were removed after it: never offered. */
  left: number;
  onClose: () => void; onError: (e: unknown) => void;
}) {
  useEscape(onClose);
  const key = (r: BulkRow) => r.repo + "|" + r.slug;
  const [on, setOn] = useState<Set<string>>(() => new Set(rows.filter((r) => bulkDefault(r, now)).map(key)));
  const [done, setDone] = useState<Record<string, string>>({});
  const [working, setWorking] = useState(false);
  const total = rows.filter((r) => on.has(key(r))).reduce((a, r) => a + r.frees, 0);
  const run = async () => {
    setWorking(true);
    const roots = new Set<string>();
    for (const r of rows.filter((x) => on.has(key(x)))) {
      let freed = 0;
      const why: string[] = [];
      for (const c of r.items) {
        try { const d = await disk.del({ repo: r.repo, slug: r.slug, path: c.path, id: c.id, placePath: r.path, label: `${r.slug}/${c.path}` }); freed += d.bytes; } catch (e) { why.push(String(e)); }
      }
      roots.add(r.repo);
      setDone((m) => ({ ...m, [key(r)]: why.length ? why[0] : `freed ${gb(freed)}` }));
    }
    setWorking(false);
    void disk.scan([...roots], true).catch(onError);
  };
  const finished = Object.keys(done).length > 0 && !working;
  return (
    <div className="scrim scrim-center" onClick={onClose}>
      <div className="sync-modal dk-bulk" role="dialog" aria-modal="true" aria-label="Clean up build output" onClick={(e) => e.stopPropagation()}>
        <header className="sync-h"><b>Clean up build output</b></header>
        <div className="sync-body">
          <div className="dk-hint">Build folders by name (target, .next, build, DerivedData…), and only where git ignores them. Source, dependencies and git are not touched.</div>
          {left > 0 && <div className="dk-hint" data-testid="disk-bulk-left">{left === 1 ? "1 place was" : `${left} places were`} removed after the scan and {left === 1 ? "is" : "are"} left out.</div>}
          <div className="dk-tbl">
            <div className="dk-th"><span /><span>Place</span><span>Last commit</span><span>Agent</span><span className="num">Frees</span></div>
            {rows.map((r) => (
              <label className="dk-tr" key={key(r)} title={r.items.map((c) => c.path).join(", ")}>
                <input type="checkbox" checked={on.has(key(r))} disabled={working || !!done[key(r)]}
                  onChange={(e) => setOn((s) => { const n = new Set(s); if (e.target.checked) n.add(key(r)); else n.delete(key(r)); return n; })} />
                <span><b>{r.slug}</b> <small>{r.project}</small>{done[key(r)] && <small className="dk-done"> · {done[key(r)]}</small>}</span>
                <span><small>{ago(now, r.last)}</small></span>
                <span>{r.busy ? <small className="dk-live">working</small> : <small>—</small>}</span>
                <span className="num">{gb(r.frees)}</span>
              </label>
            ))}
            {!rows.length && <div className="dk-tr"><span /><span className="dk-hint">No place holds more than 100 MB of build output.</span></div>}
          </div>
        </div>
        <footer className="sync-foot">
          <span className="dk-hint dk-foot-note">Ticked by default: places idle for two weeks with no agent working.</span>
          <button className="ctrl" onClick={onClose}>{finished ? "Done" : "Cancel"}</button>
          {!finished && (
            <button className="enter-btn danger" data-testid="disk-bulk-go" disabled={working || total === 0} onClick={() => void run()}>
              {working ? "Deleting…" : `Delete ${gb(total)}`}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}

export type { Cleanable };

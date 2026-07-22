// Browser design harness: install a fake `window.__TAURI_INTERNALS__` so the real
// App.tsx runs in a plain browser (Vite) with a mocked, STATEFUL backend + rich
// fixtures. Loaded only when VITE_MOCK=1 (see main.tsx). Never bundled in prod.
//
// The command names here MUST track the real Tauri handlers (lib.rs). Unknown
// commands resolve to null + a console.warn, so new commands never hard-crash the
// harness during a redesign.

import { initialWorkspace, type Place, type Workspace } from "./fixtures";

let ws: Workspace = initialWorkspace();

const clone = <T>(x: T): T => JSON.parse(JSON.stringify(x));
const now = () => Math.floor(Date.now() / 1000);

function findProject(root: string) {
  return ws.projects.find((p) => p.root === root);
}
function editPlace(repo: string, slug: string, fn: (p: Place) => void) {
  const pv = findProject(repo);
  const pl = pv?.snapshot?.places.find((p) => p.slug === slug);
  if (pl) fn(pl);
}
function reconcile(pl: Place) {
  const life = pl.declared?.lifecycle;
  if (life === "archived" || life === "abandoned") pl.lifecycle_effective = life;
  else if (life === "saved") pl.lifecycle_effective = "saved";
  else if (pl.tmux_session.up) pl.lifecycle_effective = "active";
  else if (pl.declared?.last_opened_epoch && now() - pl.declared.last_opened_epoch < 7 * 86400)
    pl.lifecycle_effective = "idle";
  else pl.lifecycle_effective = "closed";
}

let dialogCount = 0;

type Args = Record<string, any>;
async function mockInvoke(cmd: string, args: Args = {}): Promise<unknown> {
  switch (cmd) {
    case "list_workspace":
      return clone(ws);
    case "list_places":
      return clone(findProject(args.repo)?.snapshot ?? null);

    case "plugin:dialog|open": {
      // simulate a native folder pick → a fresh project path
      dialogCount += 1;
      return `/Users/demo/workspace/picked-${dialogCount}`;
    }
    case "add_project": {
      const root: string = args.dir;
      if (!findProject(root)) {
        const name = root.split("/").pop() || root;
        ws.projects.push({
          root, ok: true, error: null,
          snapshot: {
            repo: root, prefix: name,
            places: [{
              slug: "(main)", path: root, is_main: true, registered: true,
              branch: "main", detached: false, dirty: false, dirty_files: 0,
              ahead: 0, behind: 0, last_commit_subject: "initial commit",
              last_commit_epoch: now() - 86400,
              tmux_session: { name: `${name}-main`, up: false },
              claude_session_present: false, declared: null, lifecycle_effective: "closed",
            }],
          },
        });
      }
      return clone(ws);
    }
    case "remove_project":
      ws.projects = ws.projects.filter((p) => p.root !== args.root);
      return clone(ws);

    case "set_lifecycle":
      editPlace(args.repo, args.slug, (p) => {
        p.declared = { ...(p.declared ?? {}), lifecycle: args.label };
        reconcile(p);
      });
      return null;
    case "set_pin":
      editPlace(args.repo, args.slug, (p) => {
        p.declared = { ...(p.declared ?? {}), pinned: args.on };
      });
      return null;
    case "set_note":
      editPlace(args.repo, args.slug, (p) => {
        p.declared = { ...(p.declared ?? {}), note: args.note || undefined };
      });
      return null;
    case "touch_place":
      editPlace(args.repo, args.slug, (p) => {
        p.declared = { ...(p.declared ?? {}), last_opened_epoch: now() };
        reconcile(p);
      });
      return null;

    case "new_place": {
      const pv = findProject(args.repo);
      const slug = (args.name || args.branch).replace(/\//g, "-");
      if (pv?.snapshot && !pv.snapshot.places.find((p) => p.slug === slug)) {
        pv.snapshot.places.push({
          slug, path: `${args.repo}/.worktrees/${slug}`, is_main: false, registered: true,
          branch: args.branch, detached: false, dirty: false, dirty_files: 0,
          ahead: 0, behind: 0, last_commit_subject: "wip", last_commit_epoch: now(),
          tmux_session: { name: `${pv.snapshot.prefix}-${slug}`, up: true },
          claude_session_present: true,
          declared: { last_opened_epoch: now() }, lifecycle_effective: "active",
        });
      }
      return { ok: true, code: 0, output: `Created worktree ${slug}` };
    }
    case "open_place": {
      editPlace(args.repo, args.slug, (p) => {
        p.tmux_session.up = true;
        p.claude_session_present = true;
        p.declared = { ...(p.declared ?? {}), last_opened_epoch: now() };
        reconcile(p);
      });
      return { ok: true, code: 0, output: `Opened ${args.slug}` };
    }
    case "switch_place":
      editPlace(args.repo, args.slug, (p) => { p.branch = args.branch; });
      return { ok: true, code: 0, output: `Switched ${args.slug} → ${args.branch}` };
    case "remove_place": {
      const pv = findProject(args.repo);
      if (pv?.snapshot) pv.snapshot.places = pv.snapshot.places.filter((p) => p.slug !== args.slug);
      return { ok: true, code: 0, output: `Removed ${args.slug}` };
    }

    case "term_open": {
      // best-effort: render a canned banner into the xterm via the Channel
      const banner =
        "\x1b[38;5;110m worktrees \x1b[0m mock terminal — design harness\r\n" +
        "\x1b[90m(real tmux attach only in the Tauri app)\x1b[0m\r\n\r\n" +
        `\x1b[32m➜\x1b[0m  \x1b[36m${args.session}\x1b[0m $ \x1b[5m▌\x1b[0m\r\n`;
      const ch = args.onBytes;
      setTimeout(() => {
        try { ch?.onmessage?.(new TextEncoder().encode(banner).buffer); } catch { /* ignore */ }
      }, 40);
      return 1;
    }
    case "term_write":
    case "term_resize":
    case "term_close":
      return null;

    // settings — harness has no persistence; App falls back to defaults
    case "get_settings":
      return null;
    case "set_settings":
      return null;

    // event system — no live emitter in the harness; resolve quietly
    case "plugin:event|listen":
      return 0;
    case "plugin:event|unlisten":
      return null;

    default:
      console.warn("[mock] unhandled command:", cmd, args);
      return null;
  }
}

// Minimal internals surface the @tauri-apps/api v2 uses.
let cbId = 0;
const callbacks: Record<number, (v: unknown) => void> = {};
(window as any).__TAURI_INTERNALS__ = {
  invoke: (cmd: string, args?: Args) => mockInvoke(cmd, args),
  transformCallback: (cb: (v: unknown) => void) => {
    const id = ++cbId;
    callbacks[id] = cb;
    return id;
  },
  convertFileSrc: (p: string) => p,
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
};

console.info("[mock] Tauri backend mocked — design harness active");

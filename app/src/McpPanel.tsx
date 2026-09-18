// Wiring the worktrees MCP server into the user's claude — Settings → Claude,
// and the passive card on Home.
//
// Both surfaces render from ONE `McpStatus`, fetched by App and passed down, so
// the card and the panel can never disagree about what is installed. The verdict
// itself is computed in `worktrees_core::mcpsetup` and shared with the CLI's
// `worktrees mcp --status`; nothing here re-derives it, and the `state` switch
// below is the only place each case's words live.
//
// What the server is FOR, in one sentence, because both surfaces have to say it:
// it lets a Claude session create, inspect and close worktrees as tools instead
// of reinventing them with raw git. The orchestrator pattern this repo runs on
// (a session in `(main)` briefing sessions in the other places) needs the
// mutating half, which is why "full" is the default the buttons offer.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import * as Icons from "./icons";

/** Mirrors `mcpsetup::Entry`. */
export type McpEntry = {
  command: string;
  args: string[];
  mutations: boolean;
  command_ok: boolean;
  ours: boolean;
};

/** Mirrors `mcpsetup::State`, kebab-cased by serde. */
export type McpState =
  | "not-applicable"
  | "installed"
  | "read-only"
  | "stale"
  | "foreign"
  | "elsewhere"
  | "absent"
  | "cli-missing";

/** Mirrors `mcpsetup::Status`. */
export type McpStatus = {
  state: McpState;
  ai_cmd: string;
  claude_bin: string | null;
  worktrees_bin: string | null;
  user: McpEntry | null;
  found_in: ("user" | "local" | "project" | "profile")[];
  command: string | null;
  config_path: string;
};

/** Mirrors `mcpsetup::Outcome`. */
export type McpOutcome = { ok: boolean; output: string; status: McpStatus };

/** The one state the passive nudge may appear for — `State::nudgeable` on the
 *  Rust side, repeated here rather than shipped as a field because it is a
 *  question about THIS UI, not about the machine. `stale` and `read-only` are
 *  deliberately not included: they are real problems, and they belong in
 *  Settings where they cannot be dismissed by someone who only ever wanted the
 *  install offer to stop. */
export const canNudge = (s: McpStatus | null) => s?.state === "absent";

/** One line for the current state, plus how alarmed to look. Shared by both
 *  surfaces so the Home card and the Settings panel say the same thing. */
function verdict(s: McpStatus): { tone: "ok" | "warn" | "off"; line: string } {
  switch (s.state) {
    case "installed":
      return { tone: "ok", line: "Connected — Claude can create, inspect and close worktrees here." };
    case "read-only":
      return {
        tone: "warn",
        line: "Connected, but read-only. Claude can look at your worktrees and cannot create or close one.",
      };
    case "stale":
      return {
        tone: "warn",
        line: `Configured, but the binary it points at is gone (${s.user?.command ?? "?"}). Every Claude session fails to start it.`,
      };
    case "foreign":
      return {
        tone: "warn",
        line: `Another MCP server is already registered under the name “worktrees” (it runs ${s.user?.command ?? "?"}). Nothing here will touch it.`,
      };
    case "elsewhere":
      return { tone: "ok", line: `Already configured (${s.found_in.join(", ")}) — nothing to do.` };
    case "absent":
      return { tone: "off", line: "Not set up. Claude has no way to drive your worktrees directly." };
    case "cli-missing":
      return {
        tone: "off",
        line: "The worktrees CLI is not installed, and the server needs it. Install the CLI under Updates first.",
      };
    case "not-applicable":
      return { tone: "off", line: `Your AI command is ${s.ai_cmd}, not claude — this does not apply.` };
  }
}

/** Settings → Claude. The permanent home: the real state, the actions, and the
 *  command spelled out for anyone who would rather run it themselves. */
export function McpSection({ status, repo, onChanged, onReport }: {
  status: McpStatus | null;
  /** The project in focus, or "" — only used to check the local/project scopes. */
  repo: string;
  onChanged: (s: McpStatus) => void;
  onReport: (msg: string) => void;
}) {
  // App's startup probe runs with NO repo — it is for the Home card, which is a
  // machine-level offer and has no project in focus. The local and project
  // scopes can only be consulted with one, so the panel re-reads on mount with
  // the repo in hand. ON DEMAND, never on a timer: the same discipline `doctor`
  // keeps, for the same reason — nothing here belongs on the poll path.
  useEffect(() => {
    invoke<McpStatus>("mcp_status", { repo: repo || null }).then(onChanged).catch(() => {});
    // `onChanged` is App's setState, stable; re-running on it would loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo]);

  const [busy, setBusy] = useState<"" | "install" | "remove">("");
  const [log, setLog] = useState("");
  const [mutations, setMutations] = useState(true);
  const [copied, setCopied] = useState(false);
  const [removeArmed, setRemoveArmed] = useState(false);

  const run = async (kind: "install" | "remove") => {
    if (busy) return;
    setBusy(kind);
    setLog("");
    try {
      const cmd = kind === "install" ? "mcp_install" : "mcp_uninstall";
      const o = await invoke<McpOutcome>(cmd, { repo: repo || null, mutations });
      setLog(o.output + (o.ok ? "\n✓ done" : "\n✗ did not take effect"));
      // The RE-READ status, never an assumption that the click worked — the same
      // rule `update_cli` follows (an installer that exits 0 without changing
      // anything is the failure that looks like success).
      onChanged(o.status);
    } catch (e) {
      setLog(`✗ ${String(e)}`);
      onReport(String(e));
    } finally {
      setBusy("");
      setRemoveArmed(false);
    }
  };

  const copy = () => {
    if (!status?.command) return;
    if (!navigator.clipboard) { onReport("clipboard unavailable"); return; }
    navigator.clipboard
      .writeText(status.command)
      .then(() => { setCopied(true); setTimeout(() => setCopied(false), 1400); })
      .catch((e) => onReport(String(e)));
  };

  if (!status) return <section className="setting"><label>Claude MCP server</label><div className="hint">Checking…</div></section>;
  const v = verdict(status);
  const installed = status.state === "installed" || status.state === "read-only" || status.state === "stale";
  // `foreign` is the one state with a user-scope entry and no button: we do not
  // delete a server we did not write, and core refuses it anyway.
  const canAct = status.state === "absent" || status.state === "stale" || status.state === "read-only";
  const verb = status.state === "stale" ? "Repair" : status.state === "read-only" ? "Re-install" : "Set up";

  return (
    <section className="setting">
      <label>
        Claude MCP server
        {status.state === "stale" && <span className="upd-tag">broken</span>}
        {status.state === "absent" && <span className="upd-tag">not set up</span>}
      </label>
      <div className="hint">
        Lets a Claude session manage worktrees as tools — list them, read their status, create and close
        them — instead of driving git by hand. Installed once, for every project: the server works out
        which repo it is in from the session's directory.
      </div>

      <div className={"mcp-verdict mcp-" + v.tone}>
        {v.tone === "ok" ? <Icons.Check size={14} /> : <Icons.TriangleAlert size={14} />}
        <span>{v.line}</span>
      </div>

      {installed && status.user && (
        <div className="ver-rows">
          <div className="ver-row">
            scope <b>user</b> · <span className="ver-path" title={status.user.command}>{status.user.command}</span>
          </div>
          <div className="ver-row">
            tools <b>{status.user.mutations ? "read + write" : "read-only"}</b>
            {status.user.mutations && " — create, close and remove worktrees"}
          </div>
        </div>
      )}

      {canAct && (
        <>
          <label className="tier-toggle setting-check">
            <input type="checkbox" checked={mutations} onChange={(e) => setMutations(e.currentTarget.checked)} />
            Let Claude create and close worktrees
          </label>
          {/* Said plainly, because the guard is thinner than it looks: the
              destructive tools ask for a `confirm: true` argument, and the model
              is the one that sets it. What actually holds is that it happens in
              a pane you are watching. */}
          <div className="hint">
            Unticked, Claude can only read. Ticked, it can also create worktrees and close or remove them —
            removing one discards uncommitted work in it. Turn this off if you want to watch before you trust it.
          </div>
        </>
      )}

      <div className="ver-actions">
        {canAct && (
          <button className="ctrl sm" disabled={!!busy || !status.claude_bin} onClick={() => run("install")}>
            {busy === "install" ? "Working…" : `${verb}${status.claude_bin ? "" : " (claude not found)"}`}
          </button>
        )}
        {status.command && (
          <button className="ctrl sm" onClick={copy}>{copied ? "Copied" : "Copy command"}</button>
        )}
        {installed && status.state !== "foreign" && (
          <button
            className={"ctrl sm danger" + (removeArmed ? " armed" : "")}
            disabled={!!busy || !status.claude_bin}
            onClick={() => (removeArmed ? run("remove") : setRemoveArmed(true))}
          >
            {busy === "remove" ? "Removing…" : removeArmed ? "Remove?" : "Remove"}
          </button>
        )}
      </div>

      {status.command && (
        <>
          <div className="hint">
            {status.claude_bin
              ? "The button runs exactly this — we never edit Claude's config file ourselves:"
              : "`claude` is not on this app's PATH, so run this yourself in a terminal:"}
          </div>
          <pre className="update-log mcp-cmd">{status.command}</pre>
        </>
      )}

      {log && <pre className="update-log">{log}</pre>}
      <div className="hint">Servers are recorded in {status.config_path}. Restart a Claude session for a change to reach it.</div>
    </section>
  );
}

/** The passive card on Home. Only ever appears for `absent` (see `canNudge`),
 *  only once there is a project to use it on, and never for a machine whose AI
 *  command is not claude.
 *
 *  There is no "not now" button, deliberately: ignoring it IS "not now". It sits
 *  on Home, never over the terminal, so it costs nothing to leave standing — and
 *  a third button would ask the user to distinguish two kinds of silence when
 *  only one of them is a decision. "Don't show again" is the persisted flag, and
 *  the card retires itself the moment the server exists, so that flag only ever
 *  governs this one case. */
export function McpNudge({ status, onOpenSettings, onDismiss }: {
  status: McpStatus;
  onOpenSettings: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="mcp-card">
      <div className="mcp-card-h">
        <Icons.SquareTerminal size={14} />
        Let Claude drive your worktrees
      </div>
      <p>
        Claude can create, inspect and close worktrees as tools instead of running git by hand — one
        setup, every project. {status.claude_bin ? "We can wire it up for you." : "Settings has the command to run."}
      </p>
      <div className="ver-actions">
        <button className="ctrl sm" onClick={onOpenSettings}>Set up…</button>
        <button className="mcp-dismiss" onClick={onDismiss}>Don’t show again</button>
      </div>
    </div>
  );
}

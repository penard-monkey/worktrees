// Settings → General (settings-reorg §3.2): how this window starts, the two
// commands it hands files and sessions to, pull requests, the background
// fetch (the host's), and where new projects go.
import { useEffect, useState } from "react";
import { invoke } from "../ipc";
import { pickDirForHost, hostOnly } from "../onhost";
import { FetchIntervalSection } from "../HostPolicyControls";
import { Page, Sec, useSheet } from "./kit";

/** One row of `detect_editors` (lib.rs `editors::Editor`). */
type DetectedEditor = { label: string; cmd: string; kind: "app" | "cli" };
/** The select's Custom… value — not a command anyone could store. */
const EDITOR_CUSTOM = "\u0000custom";

export function GeneralPage() {
  const { settings, onChange, onReport, onFail } = useSheet();
  /** Installed editors (lib.rs `detect_editors`); null until asked. */
  const [editors, setEditors] = useState<DetectedEditor[] | null>(null);
  /** "Custom…" was CHOSEN. Without it, picking Custom while the stored command
   *  matches a detected entry would snap straight back to that entry. */
  const [editorCustom, setEditorCustom] = useState(false);
  useEffect(() => {
    let alive = true;
    invoke<DetectedEditor[]>("detect_editors")
      .then((list) => { if (alive) setEditors(list); })
      .catch((e) => { if (alive) onReport(String(e)); });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- probe on entry, not every App render
  }, []);
  const pickParent = async () => {
    try {
      const d = await pickDirForHost("Where new projects go");
      if (d) onChange({ projects_parent: d });
    } catch (e) { onReport(String(e)); }
  };
  // `editor_cmd` stays the ONE stored value: a pick writes its command there,
  // and a stored command that matches no detected entry simply reads as Custom.
  const known = editors?.some((e) => e.cmd === settings.editor_cmd) ?? false;
  const custom = editors === null || editors.length === 0 || editorCustom || !known;
  return <Page page="general">
    <Sec id="restore-last">
      <section className="setting">
        <label>Startup</label>
        <label className="tier-toggle setting-check">
          <input type="checkbox" checked={settings.restore_last}
            onChange={(e) => onChange({ restore_last: e.currentTarget.checked })} />
          Restore last place on launch
        </label>
        <div className="hint">Reopens the most recently used place, ready but not started — press Enter to attach.</div>
      </section>
    </Sec>

    <Sec id="restore-window">
      <section className="setting">
        <label>Restore window</label>
        <label className="tier-toggle setting-check">
          <input type="checkbox" checked={settings.restore_window}
            onChange={(e) => onChange({ restore_window: e.currentTarget.checked })} />
          Restore window on launch
        </label>
        <div className="hint">
          Reopens the window the way you left it — size, position, and full screen — after a
          quit or an update. Takes effect the next time the app starts.
        </div>
      </section>
    </Sec>

    <Sec id="editor">
      <section className="setting">
        <label>Editor</label>
        {editors !== null && editors.length > 0 && (
          <select
            value={custom ? EDITOR_CUSTOM : settings.editor_cmd}
            onChange={(e) => {
              const v = e.currentTarget.value;
              if (v === EDITOR_CUSTOM) { setEditorCustom(true); return; }
              setEditorCustom(false);
              onChange({ editor_cmd: v });
            }}
          >
            {editors.map((e) => <option key={e.cmd} value={e.cmd}>{e.label}</option>)}
            <option value={EDITOR_CUSTOM}>Custom…</option>
          </select>
        )}
        {custom && (
          <input
            type="text" value={settings.editor_cmd}
            aria-label="Editor command"
            // Typing IS choosing Custom. Without this, a field shown only
            // because the stored value matched nothing unmounts — focused,
            // mid-word — the keystroke its text first equals a detected
            // entry's command.
            onChange={(e) => { setEditorCustom(true); onChange({ editor_cmd: e.currentTarget.value }); }}
          />
        )}
        <div className="hint">
          Used by right-click “Open in editor” and ⌘E.
          {editors !== null && editors.length === 0 && " No known editor was found in Applications or on your PATH."}
          {" "}A custom command takes quoted args (e.g. code, cursor, open -a "Visual Studio Code"); the file is added as the last argument.
        </div>
      </section>
    </Sec>

    <Sec id="terminal-command">
      <section className="setting">
        <label>Terminal command</label>
        <input type="text" value={settings.terminal_cmd} aria-label="Terminal command"
          onChange={(e) => onChange({ terminal_cmd: e.currentTarget.value })} />
        <div className="hint">Right-click “Open in terminal app”. {"{session}"} is the tmux session name, already shell-quoted (e.g. ghostty -e tmux attach -t {"{session}"}). Leave empty to hide the item.</div>
      </section>
    </Sec>

    <Sec id="pull-requests">
      <section className="setting">
        <label>Pull requests</label>
        <label className="tier-toggle setting-check">
          <input type="checkbox" checked={settings.pull_requests}
            onChange={(e) => onChange({ pull_requests: e.currentTarget.checked })} />
          Show pull requests for GitHub projects
        </label>
        <div className="hint">
          A chip in a place's header for its branch's PR, a Pull requests tab in the dock, and a
          count on its icon. Read through the GitHub CLI (<code>gh</code>) for the selected
          project only, while the window is visible. Off stops every <code>gh</code> call.
        </div>
      </section>
    </Sec>

    <Sec id="auto-fetch">
      <FetchIntervalSection settings={settings} onChange={onChange} onFail={onFail} />
    </Sec>

    <Sec id="projects-parent">
      <section className="setting">
        <label>New projects</label>
        <div className="row2">
          <input type="text" value={settings.projects_parent} placeholder="the folder your projects share, else ~/workspace"
            aria-label="Folder new projects go in" spellCheck={false}
            onChange={(e) => onChange({ projects_parent: e.currentTarget.value })} />
          <button type="button" className="ctrl sm" onClick={() => void pickParent()} {...hostOnly("Choosing a folder")}>Choose…</button>
        </div>
        <div className="hint">Where New project… and Clone from URL… put a project. Each of them also remembers the folder it last used here. Empty: the folder your projects already share, else ~/workspace.</div>
      </section>
    </Sec>
  </Page>;
}

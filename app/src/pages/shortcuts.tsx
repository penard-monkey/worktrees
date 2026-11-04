// Settings → Shortcuts: a reference card, last (settings-reorg §3.2).
import { Page, Sec } from "./kit";

export function ShortcutsPage() {
  return <Page page="shortcuts">
    <Sec id="shortcuts">
      <section className="setting">
        <label>Shortcuts</label>
        <div className="shortcuts">
          {[
            ["⌘B", "Sidebar — reveal, then pin (unpin when pinned)"],
            ["⌘J", "Toggle the dock (Files / Terminal)"],
            ["⌘K", "Quick switcher"],
            ["⌘,", "Open Settings"],
            ["⌘1", "Home (briefing)"],
            ["⌘2", "Places — reveal the sidebar and focus the filter"],
            ["⌘E", "Open selection in editor"],
            ["⌘F", "Find — in the terminal, or in the open file"],
            ["⌘S", "Save the markdown file you are editing in Source view"],
            ["⌘B / ⌘I", "Bold / italic — while the markdown editor has focus (⌘B is the sidebar everywhere else)"],
            ["⌘T", "New terminal in the dock"],
            ["⌘N", "New worktree — in the current project, or pick one"],
            ["⌘⇧E", "Read the dock's file over the whole pane"],
            ["⌘+ / ⌘−", "Overall size — the whole window, terminal included"],
            ["⌘0", "Overall size back to 100%"],
            ["⌘⌥+ / ⌘⌥−", "Reading size of a rendered markdown file"],
            ["Esc", "Close sheets, menus & the revealed sidebar"],
          ].map(([key, desc]) => (
            <div className="shortcut-row" key={key}>
              <kbd>{key}</kbd>
              <span>{desc}</span>
            </div>
          ))}
        </div>
      </section>
    </Sec>
  </Page>;
}

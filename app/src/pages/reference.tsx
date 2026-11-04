// Settings → Reference panels (settings-reorg §3.2, §6 item 5): the Files
// tab's preferences, which until now only its own toolbar could change — and
// `files_show_ignored` once hid every file a session wrote, with a toolbar icon
// as the only switch. The keys are the toolbar's own; nothing new is stored.
import { MD_ZOOM_STEPS, clampMdZoom, type Settings } from "../settings";
import { Page, Sec, useSheet } from "./kit";

/** The divider's range, the same 15–85 the Files tab's drag clamps to. */
const PCT_MIN = 15;
const PCT_MAX = 85;
const clampPct = (v: number) => Math.max(PCT_MIN, Math.min(PCT_MAX, Math.round(v)));

/** A yes/no as two words, so the choice reads without a checkbox's double
 *  negative ("show ignored: off"). */
function Pair<K extends keyof Settings>({ k, on, off, testid }: { k: K; on: string; off: string; testid: string }) {
  const { settings, onChange } = useSheet();
  const v = settings[k] as unknown as boolean;
  return (
    <div className="seg seg-plain" data-testid={testid}>
      <button className={!v ? "on" : ""} onClick={() => onChange({ [k]: false } as Partial<Settings>)}>{off}</button>
      <button className={v ? "on" : ""} onClick={() => onChange({ [k]: true } as Partial<Settings>)}>{on}</button>
    </div>
  );
}

export function ReferencePage() {
  const { settings, onChange } = useSheet();
  return <Page page="reference">
    <Sec id="files-layout">
      <section className="setting">
        <label>Files layout</label>
        <div className="seg seg-plain" data-testid="files-layout">
          {([["auto", "Automatic"], ["stack", "Stacked"], ["split", "Side by side"]] as const).map(([v, label]) => (
            <button key={v} className={settings.files_layout === v ? "on" : ""} onClick={() => onChange({ files_layout: v })}>{label}</button>
          ))}
        </div>
        <div className="hint">Where the file tree sits against the file you are reading. Automatic puts them side by side once the panel is wide enough, and stacks them when it is not.</div>
        <label className="sub">Tree's share, side by side <span className="val">{settings.files_split_pct}%</span></label>
        <input type="range" min={PCT_MIN} max={PCT_MAX} step={1} value={clampPct(settings.files_split_pct)} aria-label="Tree's share side by side"
          onChange={(e) => onChange({ files_split_pct: clampPct(+e.currentTarget.value) })} />
        <label className="sub">Tree's share, stacked <span className="val">{settings.files_stack_pct}%</span></label>
        <input type="range" min={PCT_MIN} max={PCT_MAX} step={1} value={clampPct(settings.files_stack_pct)} aria-label="Tree's share stacked"
          onChange={(e) => onChange({ files_stack_pct: clampPct(+e.currentTarget.value) })} />
        <div className="hint">Dragging the divider in the Files tab sets these too.</div>
      </section>
    </Sec>

    <Sec id="files-reading">
      <section className="setting">
        <label>Reading</label>
        <label className="sub">Long lines in source</label>
        <Pair k="files_wrap" off="Scroll sideways" on="Wrap" testid="files-wrap" />
        <label className="sub">Markdown and SVG files</label>
        <Pair k="files_md_source" off="Rendered" on="Source" testid="files-md-source" />
        <label className="sub">Markdown reading size <span className="val">{clampMdZoom(settings.files_md_zoom)}%</span></label>
        <select value={clampMdZoom(settings.files_md_zoom)} aria-label="Markdown reading size" data-testid="files-md-zoom"
          onChange={(e) => onChange({ files_md_zoom: clampMdZoom(+e.currentTarget.value) })}>
          {MD_ZOOM_STEPS.map((n) => <option key={n} value={n}>{n}%</option>)}
        </select>
        <div className="hint">The size a place starts at. A place where you have set its own size with ⌘⌥+ / ⌘⌥− keeps it.</div>
      </section>
    </Sec>

    <Sec id="files-tree">
      <section className="setting">
        <label>File tree</label>
        <label className="sub">Files git ignores</label>
        <Pair k="files_show_ignored" off="Hide" on="Show, dimmed" testid="files-show-ignored" />
        <label className="sub">Which files</label>
        <Pair k="files_changed_only" off="Every file" on="Only what the branch changed" testid="files-changed-only" />
        <div className="hint">Both default to showing everything: a filter you forget you left on looks exactly like a repo that lost its files.</div>
      </section>
    </Sec>

    <Sec id="files-diff">
      <section className="setting">
        <label>Diffs</label>
        <div className="seg seg-plain" data-testid="files-diff-base">
          {([["base", "What the branch changed"], ["head", "What is not committed"]] as const).map(([v, label]) => (
            <button key={v} className={settings.files_diff_base === v ? "on" : ""} onClick={() => onChange({ files_diff_base: v })}>{label}</button>
          ))}
        </div>
        <div className="hint">What a file's diff shows: the branch's whole change since its base, or only what is not committed yet. The tree's change markers count from the base, so that is the default.</div>
      </section>
    </Sec>
  </Page>;
}

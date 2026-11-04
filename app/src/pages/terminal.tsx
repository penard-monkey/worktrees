// Settings → Terminal (settings-reorg §3.2): how terminals look and how much
// they keep, then what a dock shell tab remembers — the host's, since every
// window that opens a tab shares its shell — and the saved history itself,
// beside the switch that explains it (it used to sit under Data).
import { useEffect, useState } from "react";
import { invoke } from "../ipc";
import { humanSize } from "../filekind";
import { clampScrollback, clampTerm, TERM_SCROLLBACK_STEPS } from "../settings";
import { TermHistorySection } from "../HostPolicyControls";
import { useArmed } from "../useArmed";
import { Page, Sec, useSheet } from "./kit";

/** `term_history_info` — what the saved-scrollback tree currently costs. */
type TermHistoryInfo = { dir: string; bytes: number; tabs: number };

function SavedHistory() {
  const { open, onReport } = useSheet();
  const [info, setInfo] = useState<TermHistoryInfo | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const arm = useArmed<"clear">();
  useEffect(() => {
    if (!open) return;
    invoke<TermHistoryInfo>("term_history_info").then(setInfo).catch((e) => setErr(String(e)));
  }, [open]);
  // The one store here that holds user CONTENT rather than preferences, so it
  // gets its own size readout and its own two-click clear.
  const clear = () => arm.confirm("clear", async () => {
    try {
      await invoke("term_history_clear");
      setInfo(await invoke<TermHistoryInfo>("term_history_info"));
      setErr(null);
    } catch (e) {
      const m = `clear terminal history failed: ${String(e)}`;
      setErr(m);
      onReport(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  });
  return (
    <section className="setting">
      <label>Saved terminal history</label>
      <div className="ver-rows">
        <div className="ver-row">
          <span className="ver-path" title={info?.dir}>
            {info ? `${info.tabs} tab(s), ${humanSize(info.bytes)}` : "…"}
          </span>
        </div>
      </div>
      <div className="ver-actions">
        <button className={"ctrl sm danger" + (arm.armed ? " armed" : "")} data-testid="term-history-clear" onClick={clear}>
          {arm.armed ? "Confirm clear?" : "Clear terminal history"}
        </button>
      </div>
      {arm.armed && <div className="hint">Forgets every tab's saved output and command history. Terminals already on screen keep what they are showing.</div>}
      {err && <pre className="update-log">{err}</pre>}
    </section>
  );
}

export function TerminalPage() {
  const { settings, onChange, onFail } = useSheet();
  const lines = clampScrollback(settings.term_scrollback);
  return <Page page="terminal">
    <Sec id="term-font">
      <section className="setting">
        <label>Terminal font</label>
        <input type="text" value={settings.term_family} aria-label="Terminal font"
          onChange={(e) => onChange({ term_family: e.currentTarget.value })} />
        <label className="sub">Terminal size <span className="val">{settings.term_size}px</span></label>
        <input
          type="range" min={10} max={24} step={1} value={settings.term_size} aria-label="Terminal size"
          onChange={(e) => onChange({ term_size: clampTerm(+e.currentTarget.value) })}
        />
      </section>
    </Sec>

    <Sec id="term-scrollback">
      <section className="setting">
        <label>Scrollback</label>
        <select value={lines} aria-label="Scrollback lines" data-testid="term-scrollback"
          onChange={(e) => onChange({ term_scrollback: clampScrollback(+e.currentTarget.value) })}>
          {TERM_SCROLLBACK_STEPS.map((n) => <option key={n} value={n}>{n.toLocaleString()} lines</option>)}
        </select>
        <div className="hint">
          How far back each terminal on screen keeps its output, for scrolling and ⌘F. More costs
          memory per terminal. Never fewer than a dock shell's saved output replays, so a restored
          tab always comes back whole.
        </div>
      </section>
    </Sec>

    <Sec id="term-remembers">
      <TermHistorySection settings={settings} onChange={onChange} onFail={onFail} />
    </Sec>

    <Sec id="term-history">
      <SavedHistory />
    </Sec>
  </Page>;
}

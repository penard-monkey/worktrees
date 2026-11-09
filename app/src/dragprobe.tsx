// A hand-test probe: can a drag cross from one of the app's windows to the
// other? (Ideas phase 0; David's question.) The app's own drags are pointer-
// event drags (dnd.ts), which cannot leave a window; only the HTML5 drag-and-
// drop API can. So this is one draggable chip and one drop target per window,
// on HTML5 drag events with a custom `dataTransfer` type, each step logged
// through `log_event` so the answer can be read from serve.log afterwards
// (`grep dragprobe ~/.cache/worktrees/serve.log`).
//
// DEV builds only, and only when asked for: `VITE_DRAGPROBE=1` in the dev
// server's environment, or `?dragprobe` on the page. Both callers import this
// module under `import.meta.env.DEV`, which a production build makes the
// literal `false`, so the probe is not in a release bundle at all
// (companion-boundary-check.mjs greps app/dist for its marker).
import { useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "./ipc";

/** The custom type the chip carries; nothing but this probe ever sets it. */
export const PROBE_TYPE = "application/x-worktrees-dragprobe";

function wanted(): boolean {
  return import.meta.env.VITE_DRAGPROBE === "1" || new URLSearchParams(window.location.search).has("dragprobe");
}

function log(win: string, msg: string) {
  const line = `dragprobe ${win}: ${msg}`;
  console.info(line);
  invoke("log_event", { level: "info", msg: line }).catch(() => {});
}

const types = (dt: DataTransfer | null) => (dt ? `[${[...dt.types].join(", ")}]` : "[no dataTransfer]");

function Probe({ win }: { win: string }) {
  const [last, setLast] = useState("idle");
  // One dragenter line per drag over the target, not one per child crossed.
  const entered = useRef(false);
  const note = (msg: string) => {
    setLast(msg);
    log(win, msg);
  };
  return (
    <div
      data-dragprobe=""
      style={{
        position: "fixed", right: 12, bottom: 12, zIndex: 2147483000, display: "flex", gap: 8, alignItems: "center",
        padding: 8, borderRadius: 6, background: "var(--bg-panel)", border: "1px solid var(--line-strong)",
        color: "var(--txt)", font: "12px var(--font-ui)", maxWidth: 420,
      }}
    >
      <span
        draggable
        onDragStart={(e) => {
          const payload = JSON.stringify({ from: win, at: Date.now() });
          e.dataTransfer.setData(PROBE_TYPE, payload);
          e.dataTransfer.setData("text/plain", `dragprobe from ${win}`);
          e.dataTransfer.effectAllowed = "copy";
          note(`dragstart from ${win} ${types(e.dataTransfer)}`);
        }}
        onDragEnd={(e) => note(`dragend in ${win}: dropEffect=${e.dataTransfer.dropEffect}`)}
        style={{ padding: "4px 8px", borderRadius: 4, background: "var(--bg-elev)", cursor: "grab", userSelect: "none" }}
      >
        drag me ({win})
      </span>
      <span
        onDragEnter={(e) => {
          e.preventDefault();
          if (entered.current) return;
          entered.current = true;
          note(`dragenter on ${win}'s target ${types(e.dataTransfer)}`);
        }}
        onDragOver={(e) => {
          e.preventDefault();
          e.dataTransfer.dropEffect = "copy";
        }}
        onDragLeave={() => {
          entered.current = false;
        }}
        onDrop={(e) => {
          e.preventDefault();
          entered.current = false;
          const data = e.dataTransfer.getData(PROBE_TYPE);
          note(`drop on ${win}'s target ${types(e.dataTransfer)} ${PROBE_TYPE}=${data || "(empty)"} text=${JSON.stringify(e.dataTransfer.getData("text/plain"))}`);
        }}
        style={{ padding: "4px 8px", borderRadius: 4, border: "1px dashed var(--line-strong)", minWidth: 110, textAlign: "center" }}
      >
        drop here ({win})
      </span>
      <span style={{ color: "var(--txt-dim)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{last}</span>
    </div>
  );
}

/** Mount the probe in a root of its own, beside the page's, when asked for. */
export function mountDragProbe(win: "main" | "companion"): void {
  if (!wanted()) return;
  const host = document.createElement("div");
  document.body.appendChild(host);
  ReactDOM.createRoot(host).render(<Probe win={win} />);
  log(win, "probe mounted");
}

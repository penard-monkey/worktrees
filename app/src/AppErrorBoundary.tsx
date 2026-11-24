// The boundary above everything a window's React root draws: the app's
// (main.tsx) and the Ideas companion's (companion/main.tsx). Without it any
// uncaught throw in a render unmounts the root and leaves a blank window with
// no way back but quitting. The Files tab's `ViewErrorBoundary` still keeps a
// document's failure inside its pane; this one is for everything else.
//
// Class component because that is the only way to catch in React. Nothing is
// swallowed: the crash goes to the log (serve.log, through the one door), and
// a log that cannot be written (the engine is down) says so on the console.
import { Component, type ErrorInfo, type ReactNode } from "react";
import { invoke } from "./ipc";

type Props = { where: string; children: ReactNode };
/** A box, so a thrown `null` or `undefined` is a crash too. */
type State = { crash: { err: unknown } | null };

const said = (err: unknown) => (err instanceof Error ? err.message || String(err) : String(err));

export class AppErrorBoundary extends Component<Props, State> {
  state: State = { crash: null };

  static getDerivedStateFromError(err: unknown): State {
    return { crash: { err } };
  }

  componentDidCatch(err: unknown, info: ErrorInfo) {
    // The component stack first: serve cuts a line at 4000 bytes, and it is
    // the half that names what broke.
    const msg = `ui crash (${this.props.where}): ${said(err)}\n${info.componentStack ?? ""}\n${(err instanceof Error && err.stack) || ""}`;
    const unlogged = (e: unknown) => console.error(`[crash] could not log it: ${String(e)}`, msg);
    try {
      invoke("log_event", { level: "error", msg }).catch(unlogged);
    } catch (e) {
      unlogged(e); // a throw out of here would unmount the root after all
    }
  }

  render() {
    const { crash } = this.state;
    if (!crash) return this.props.children;
    return (
      <div className="app-crash" role="alert">
        <h1>Something went wrong</h1>
        <p>This window stopped drawing. Agents and terminals keep running; Reload starts the window again.</p>
        <pre>{said(crash.err)}</pre>
        <button type="button" className="enter-btn" autoFocus onClick={() => window.location.reload()}>
          Reload
        </button>
      </div>
    );
  }
}

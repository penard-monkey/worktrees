import { Component, type ErrorInfo, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

// Top-level boundary above <App/>. Without it any uncaught render throw
// unmounts the root and leaves a blank window with no way back but a
// restart. This turns that class of crash into a recoverable pane.
// Class component because that is the only way to catch in React.
// Repo rule: never swallow errors — route them through applog
// (log_event here, the frontend's path into it).
export class AppErrorBoundary extends Component<
  { children: ReactNode },
  { err: Error | null }
> {
  state: { err: Error | null } = { err: null };

  static getDerivedStateFromError(err: Error) {
    return { err };
  }

  componentDidCatch(err: Error, info: ErrorInfo) {
    invoke("log_event", {
      level: "error",
      msg: `ui crash: ${err.message}\n${err.stack ?? ""}\n${info.componentStack ?? ""}`,
    }).catch(() => {});
  }

  render() {
    if (this.state.err) {
      return (
        <div className="app-crash" role="alert">
          <h1>Something went wrong</h1>
          <pre>{String(this.state.err.message || this.state.err)}</pre>
          <button type="button" onClick={() => window.location.reload()}>
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

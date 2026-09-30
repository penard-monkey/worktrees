import { invoke } from "@tauri-apps/api/core";

/** The one way the app puts text on the clipboard.
 *
 *  The backend first (`copy_text`, lib.rs), the web API only as a fallback.
 *  `navigator.clipboard.writeText` needs the click's transient activation, and
 *  macOS 27's WebKit consumes it on every `evaluateJavaScript:` — which is how
 *  Tauri delivers each event and each small channel message, terminal output
 *  included. With a session streaming, the click handler routinely runs with
 *  no activation left and the write rejects with `NotAllowedError`, a raw
 *  DOMException in the error banner. The native path needs no gesture, so it
 *  also covers a copy that awaits something first (Copy diagnostics).
 *
 *  The fallback is for where there is no native path (non-macOS, the mock
 *  harness in a browser). `app/scripts/clipboard-check.mjs` keeps every other
 *  file off `navigator.clipboard`. */
export async function copyToClipboard(text: string): Promise<void> {
  try {
    await invoke("copy_text", { text });
  } catch (native) {
    if (!navigator.clipboard) throw native;
    // Both failing on macOS means pbcopy failed AND the web API refused —
    // usually with the very NotAllowedError this helper exists to avoid, so
    // the backend's reason has to survive into the message.
    try {
      await navigator.clipboard.writeText(text);
    } catch (web) {
      throw new Error(`${String(native)}; web clipboard: ${String(web)}`);
    }
  }
}

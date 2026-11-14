// The beta switches as they are NOW, for surfaces that live outside App and
// so cannot read its settings: main.tsx's guest connections and EngineGate's
// RemoteLayer. App is the one writer (`publishGroupChat`, from
// `groupChatBeta(settings)` — beta.ts stays the only reader of the key), so
// these follow a click in Settings → General → Beta features at once, with
// no restart. Until App has loaded the settings every switch reads off.
import { useSyncExternalStore } from "react";

let groupChat = false;
const subs = new Set<(on: boolean) => void>();

/** App: the helper's answer for the settings it holds now. */
export function publishGroupChat(on: boolean): void {
  if (on === groupChat) return;
  groupChat = on;
  for (const f of [...subs]) f(on);
}

/** Call `f` with the switch now and on every change; returns the unsubscribe. */
export function onGroupChat(f: (on: boolean) => void): () => void {
  subs.add(f);
  f(groupChat);
  return () => { subs.delete(f); };
}

export function useGroupChat(): boolean {
  return useSyncExternalStore(
    (f) => { subs.add(f); return () => { subs.delete(f); }; },
    () => groupChat,
  );
}

/** The workspaces' memories as the main window holds them: what each folder has
 *  (`workspace_memory`), read again when `workspaces:memory` names it, and what
 *  was last looked at here (this machine's own: browser storage, which may be
 *  empty). Workspace chats' beta. */
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, listen } from "./ipc";
import type { MemoryView } from "./doc/memory";
import { unseen } from "./wsmemory";

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
const SEEN = "wt.memory.seen.";
const seenOf = (id: string): string | null => { try { return window.localStorage.getItem(SEEN + id); } catch { return null; } };

export type WsMemoryLive = {
  /** `undefined` until it has answered once. */
  of(id: string): MemoryView | undefined;
  error(id: string): string | null;
  refresh(id: string): Promise<void>;
  /** Make the memory (a folder, the index's template, a first commit). */
  make(id: string): Promise<void>;
  /** Changed since it was looked at. */
  unseen(id: string): boolean;
  look(id: string): void;
  /** Moves with every change of `id`'s folder. */
  tick(id: string): number;
};

export function useWsMemory(on: boolean, ids: readonly string[]): WsMemoryLive {
  const [views, setViews] = useState<Record<string, MemoryView>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [ticks, setTicks] = useState<Record<string, number>>({});
  const [, setLooked] = useState(0);
  const seq = useRef<Record<string, number>>({});

  const take = useCallback((id: string, v: MemoryView) => {
    setViews((x) => ({ ...x, [id]: v }));
    setErrors((e) => { if (!(id in e)) return e; const { [id]: _, ...rest } = e; return rest; });
    setTicks((t) => ({ ...t, [id]: (t[id] ?? 0) + 1 }));
  }, []);
  const refresh = useCallback(async (id: string) => {
    const n = (seq.current[id] ?? 0) + 1;
    seq.current[id] = n;
    try {
      const v = await invoke<MemoryView>("workspace_memory", { id });
      if (seq.current[id] === n) take(id, v);
    } catch (e) {
      if (seq.current[id] === n) setErrors((x) => ({ ...x, [id]: errText(e) }));
    }
  }, [take]);

  const key = ids.join(",");
  useEffect(() => {
    if (!on) { setViews({}); setErrors({}); return; }
    for (const id of key ? key.split(",") : []) void refresh(id);
  }, [on, key, refresh]);
  useEffect(() => {
    if (!on) return;
    const un = listen<{ id?: string }>("workspaces:memory", (e) => { if (e.payload?.id) void refresh(e.payload.id); });
    return () => { un.then((f) => f()).catch(() => {}); };
  }, [on, refresh]);

  return {
    of: (id) => views[id],
    error: (id) => errors[id] ?? null,
    refresh,
    async make(id) { take(id, await invoke<MemoryView>("workspace_memory_make", { id })); },
    unseen: (id) => unseen(views[id]?.last ?? null, seenOf(id)),
    look: useCallback((id: string) => {
      const last = views[id]?.last?.commit;
      if (!last || seenOf(id) === last) return;
      try { window.localStorage.setItem(SEEN + id, last); } catch { return; }
      setLooked((n) => n + 1);
    }, [views]),
    tick: (id) => ticks[id] ?? 0,
  };
}

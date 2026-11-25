/** The workspaces' chats as the main window holds them: each workspace's list
 *  (`workspace_chats`), what `workspaces:sessions` last said, and the acts. A
 *  list is read again whenever the push moves (a chat started, ended, waits or
 *  went back to work, here or anywhere) and after every act. */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke, listen } from "./ipc";
import type { Expect } from "./desk";
import { chatRows, type ChatRow, type WsChats, type WsSessionsEv } from "./wschats";

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

export type WsChatsLive = {
  sessions: WsSessionsEv | null;
  /** A workspace's answer: `undefined` until it has one. */
  of(id: string): WsChats | undefined;
  /** Why a workspace's chats could not be read, when they could not. */
  error(id: string): string | null;
  rows(id: string): ChatRow[];
  refresh(id: string): Promise<void>;
  /** A workspace whose chat is being started: its row, before there is one. */
  starting: string | null;
  start(id: string): Promise<{ conv: string } | { refused: string }>;
  /** Each throws in words. */
  resume(id: string, conv: string): Promise<void>;
  close(id: string, conv: string): Promise<void>;
  remove(id: string, conv: string): Promise<void>;
  rename(id: string, conv: string, title: string): Promise<void>;
};

export function useWsChats(on: boolean, ids: readonly string[]): WsChatsLive {
  const [lists, setLists] = useState<Record<string, WsChats>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [sessions, setSessions] = useState<WsSessionsEv | null>(null);
  const [expect, setExpect] = useState<Record<string, Expect>>({});
  const [starting, setStarting] = useState<string | null>(null);
  const seq = useRef<Record<string, number>>({});

  const refresh = useCallback(async (id: string) => {
    const n = (seq.current[id] ?? 0) + 1;
    seq.current[id] = n;
    try {
      const r = await invoke<WsChats>("workspace_chats", { id });
      if (seq.current[id] !== n) return;
      setLists((l) => ({ ...l, [id]: r }));
      setErrors((e) => { if (!(id in e)) return e; const { [id]: _, ...rest } = e; return rest; });
    } catch (e) {
      if (seq.current[id] === n) setErrors((x) => ({ ...x, [id]: errText(e) }));
    }
  }, []);

  useEffect(() => {
    if (!on) { setSessions(null); setLists({}); setErrors({}); return; }
    const un = listen<WsSessionsEv>("workspaces:sessions", (ev) => { if (ev.payload) setSessions(ev.payload); });
    return () => { un.then((f) => f()).catch(() => {}); };
  }, [on]);
  const key = ids.join(",");
  const pushed = useMemo(() => JSON.stringify(sessions), [sessions]);
  useEffect(() => {
    if (!on) return;
    for (const id of key ? key.split(",") : []) void refresh(id);
  }, [on, key, pushed, refresh]);

  // A push that agrees with what this window did ends the expectation.
  useEffect(() => {
    if (!sessions) return;
    setExpect((x) => {
      const kept = Object.entries(x).filter(([, e]) => e && !Object.values(lists).some((l) => l.chats.some((c) => c.id === e.conv && sessions.live.includes(c.session) === e.live)));
      return kept.length === Object.keys(x).length ? x : Object.fromEntries(kept);
    });
  }, [sessions, lists]);

  const rows = useMemo(() => {
    const out: Record<string, ChatRow[]> = {};
    for (const [id, l] of Object.entries(lists)) out[id] = chatRows(l.chats, sessions, (conv) => expect[conv] ?? null);
    return out;
  }, [lists, sessions, expect]);

  const act = useCallback(async (id: string, conv: string, cmd: string, live: boolean | null, more: Record<string, unknown> = {}) => {
    try {
      await invoke(cmd, { id, conv, ...more });
      if (live !== null) setExpect((x) => ({ ...x, [conv]: { conv, live } }));
    } catch (e) { throw errText(e); }
    finally { await refresh(id); }
  }, [refresh]);

  return {
    sessions, starting,
    of: (id) => lists[id],
    error: (id) => errors[id] ?? null,
    rows: (id) => rows[id] ?? [],
    refresh,
    async start(id) {
      setStarting(id);
      try {
        const r = await invoke<{ chat: { id: string } }>("workspace_chat_start", { id });
        setExpect((x) => ({ ...x, [r.chat.id]: { conv: r.chat.id, live: true } }));
        await refresh(id);
        return { conv: r.chat.id };
      } catch (e) {
        await refresh(id);
        return { refused: errText(e) };
      } finally { setStarting(null); }
    },
    resume: (id, conv) => act(id, conv, "workspace_chat_resume", true),
    close: (id, conv) => act(id, conv, "workspace_chat_close", false),
    remove: (id, conv) => act(id, conv, "workspace_chat_remove", null),
    rename: (id, conv, title) => act(id, conv, "workspace_chat_rename", null, { title }),
  };
}

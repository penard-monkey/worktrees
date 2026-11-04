import { useCallback, useEffect, useRef, useState } from "react";

/** How long a two-click button stays armed: SHORT, so an arm forgotten does
 *  not fire on a stray click minutes later (settings-reorg §2.5 found three
 *  arms that never expired). One number for every destructive button. */
export const ARM_MS = 4000;

/** A two-click confirm. The first `confirm(key, act)` arms `key`; a second
 *  within `ms` disarms and runs `act`. Arming another key moves the arm, so at
 *  most one button in a component is ever armed. The ONE copy of the rule —
 *  `settingsmap-check.mjs` fails on a component that grows its own timer.
 *
 *  The decision reads a ref, not a state updater: StrictMode calls an updater
 *  twice, and an act started from inside one would run twice. */
export function useArmed<K extends string = string>(ms: number = ARM_MS) {
  const [armed, setArmedState] = useState<K | null>(null);
  const cur = useRef<K | null>(null);
  const setArmed = useCallback((k: K | null) => { cur.current = k; setArmedState(k); }, []);
  useEffect(() => {
    if (armed === null) return;
    const t = setTimeout(() => setArmed(null), ms);
    return () => clearTimeout(t);
  }, [armed, ms, setArmed]);
  const confirm = useCallback((key: K, act: () => void) => {
    if (cur.current !== key) { setArmed(key); return; }
    setArmed(null);
    act();
  }, [setArmed]);
  const disarm = useCallback(() => setArmed(null), [setArmed]);
  return { armed, confirm, disarm };
}

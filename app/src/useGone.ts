import { useEffect, useRef, useState } from "react";

/** The places a plan's items name that are REALLY gone (`plans.ts`'s
 *  `missingPlaces`): each key here was missing from a list of places asked for
 *  after it was first found missing. `relist` asks for that list and resolves
 *  once it is the one on screen, or rejects. */
export function useGone(missing: readonly string[], relist: () => Promise<unknown>): ReadonlySet<string> {
  const [gone, setGone] = useState<ReadonlySet<string>>(new Set());
  const asked = useRef(new Set<string>());
  const ask = useRef(relist);
  ask.current = relist;
  const key = missing.join("\n");
  useEffect(() => {
    const fresh = key ? key.split("\n").filter((k) => !asked.current.has(k)) : [];
    if (!fresh.length) return;
    for (const k of fresh) asked.current.add(k);
    ask.current().then(
      () => setGone((g) => new Set([...g, ...fresh])),
      // It could not be asked: still unknown, and asked about again later.
      () => { for (const k of fresh) asked.current.delete(k); },
    );
  }, [key]);
  return gone;
}

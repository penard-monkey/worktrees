// An element's width as a step, from its OWN box: which of `breaks` (ascending)
// it has reached. 0 is under the first. Not a container query: `container-type`
// on the document's pane would make it the containing block of everything
// `position: fixed` inside it (the `[[` picker, the pane's menus).
//
// Measured before the first paint, so nothing is drawn at the wrong step. A
// hidden element is 0 px wide, which is not a width: it keeps its last step.
import { useLayoutEffect, useState } from "react";

export function tierOf(width: number, breaks: readonly number[]): number {
  let n = 0;
  while (n < breaks.length && width >= breaks[n]) n++;
  return n;
}

/** `breaks` must be a constant of the module that calls this. */
export function useTier(el: HTMLElement | null, breaks: readonly number[]): number {
  const [tier, setTier] = useState(breaks.length);
  useLayoutEffect(() => {
    if (!el) return;
    const look = () => {
      const w = el.getBoundingClientRect().width;
      if (w > 0) setTier(tierOf(w, breaks));
    };
    look();
    const ro = new ResizeObserver(look);
    ro.observe(el);
    return () => ro.disconnect();
  }, [el, breaks]);
  return tier;
}

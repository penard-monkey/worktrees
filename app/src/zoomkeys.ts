// Which keydown is a size chord. One answer for every window: App.tsx (main)
// and companion/zoom.ts (Ideas) both ask here, and zoom-check.mjs drives it.
//
// Size chords. ⌘+/⌘−/⌘0 is the OVERALL app size (page zoom); add ⌥ for the
// rendered markdown's reading size. `zoomDir` answers 1 / -1 / 0 (reset), or
// undefined when the key is not a size chord at all.
//
// `e.key` covers both faces of each key: ⌘+ is ⌘⇧= on a US layout ("+"), ⌘− is
// plain "-" but "_" when shifted, and the numeric keypad sends "+"/"-"
// unshifted. `e.code` is the fallback, and it is what the ⌥ variants NEED:
// macOS composes Option with the keyboard layout, so ⌥- arrives as "–" (an en
// dash) and ⌥= as "≠". Keyed on `e.key` alone, ⌘⌥+/⌘⌥− would be silently dead
// on exactly the layouts that compose — the chord fires, matches nothing, and
// the reader never resizes. `e.code` is the physical key, so it is immune.
export const ZOOM_BY_KEY: Record<string, 1 | -1 | 0> = { "+": 1, "=": 1, "-": -1, "_": -1, "0": 0 };
export const ZOOM_BY_CODE: Record<string, 1 | -1 | 0> = {
  Equal: 1, NumpadAdd: 1, Minus: -1, NumpadSubtract: -1, Digit0: 0, Numpad0: 0,
};
// `??`, not `||`: 0 is a legal direction (reset) and would fall through `||`.
export const zoomDir = (e: Pick<KeyboardEvent, "key" | "code">): 1 | -1 | 0 | undefined => ZOOM_BY_KEY[e.key] ?? ZOOM_BY_CODE[e.code];

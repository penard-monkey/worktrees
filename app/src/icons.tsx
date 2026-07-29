// Icon set — inline SVG, no dependency (CLAUDE.md: no UI libraries).
//
// Geometry is Lucide's (MIT), used at its native 24 viewBox with strokeWidth 2
// rather than transcribed down to the 16-box the two original nav icons use.
// The rendered stroke matches either way, so the set stays visually of a piece:
//
//     16-box @ 1.3 stroke, drawn 15px  →  1.3 × 15/16 = 1.22px
//     24-box @ 2.0 stroke, drawn 15px  →  2.0 × 15/24 = 1.25px
//
// ...and rescaling ~20 paths by hand is a transcription-error generator for a
// 0.03px payoff. `Home` and `Folder16` keep their hand-drawn 16-box paths.
//
// These replace the Unicode glyphs the chrome used to render (▤ ◷ ⚠ « ＋ ⚙ ▧ ★
// ⋯ ⎇ ▸ ▾ ⇅ ⌕ ✕ ✓). Those picked a different font per glyph, so stroke weight,
// optical size and baseline all disagreed — and two of them (▤ places, ▧ dock)
// were near-identical boxes bound to unrelated actions.
//
// Row-level DATA markers (◆ main, ★ pinned, ⚑ drift, ● dirty, ↑↓ ahead/behind)
// deliberately stay Unicode: they sit inline with tabular text, not on buttons.
import type { ReactNode } from "react";

type IconProps = { size?: number; className?: string };

const Svg = ({ size = 15, className, children }: IconProps & { children: ReactNode }) => (
  <svg
    className={className}
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden
  >
    {children}
  </svg>
);

// ── lenses (the activity rail) ──
/** Places — the full tree. A box (▤) said nothing about hierarchy. */
export const ListTree = (p: IconProps) => (
  <Svg {...p}>
    <path d="M21 12h-8" />
    <path d="M21 6H8" />
    <path d="M21 18h-8" />
    <path d="M3 6v4c0 1.1.9 2 2 2h3" />
    <path d="M3 10v6c0 1.1.9 2 2 2h3" />
  </Svg>
);
/** Recent — a clock with the counter-clockwise arrow; a bare clock (◷) reads
 * as "time", not "go back to what you were doing". */
export const History = (p: IconProps) => (
  <Svg {...p}>
    <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
    <path d="M3 3v5h5" />
    <path d="M12 7v5l4 2" />
  </Svg>
);
export const TriangleAlert = (p: IconProps) => (
  <Svg {...p}>
    <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" />
    <path d="M12 9v4" />
    <path d="M12 17h.01" />
  </Svg>
);

// ── panel toggles ──
// Left/right pairs that name the panel they act on. « / » never did — and the
// dock's ▧ was indistinguishable from the Places ▤ next to it.
export const PanelLeftClose = (p: IconProps) => (
  <Svg {...p}>
    <rect width="18" height="18" x="3" y="3" rx="2" />
    <path d="M9 3v18" />
    <path d="m16 15-3-3 3-3" />
  </Svg>
);
export const PanelLeftOpen = (p: IconProps) => (
  <Svg {...p}>
    <rect width="18" height="18" x="3" y="3" rx="2" />
    <path d="M9 3v18" />
    <path d="m14 9 3 3-3 3" />
  </Svg>
);
export const PanelRightClose = (p: IconProps) => (
  <Svg {...p}>
    <rect width="18" height="18" x="3" y="3" rx="2" />
    <path d="M15 3v18" />
    <path d="m8 9 3 3-3 3" />
  </Svg>
);
export const PanelRightOpen = (p: IconProps) => (
  <Svg {...p}>
    <rect width="18" height="18" x="3" y="3" rx="2" />
    <path d="M15 3v18" />
    <path d="m10 15-3-3 3-3" />
  </Svg>
);

// ── chrome ──
export const FolderPlus = (p: IconProps) => (
  <Svg {...p}>
    <path d="M12 10v6" />
    <path d="M9 13h6" />
    <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
  </Svg>
);
export const Folder = (p: IconProps) => (
  <Svg {...p}>
    <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
  </Svg>
);
export const SquareTerminal = (p: IconProps) => (
  <Svg {...p}>
    <path d="m7 11 2-2-2-2" />
    <path d="M11 13h4" />
    <rect width="18" height="18" x="3" y="3" rx="2" ry="2" />
  </Svg>
);
export const Settings = (p: IconProps) => (
  <Svg {...p}>
    <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
    <circle cx="12" cy="12" r="3" />
  </Svg>
);
/** `filled` marks the pinned state — Lucide's pin-off adds a slash that reads as
 * "pinning disabled" rather than "not pinned". */
export const Pin = ({ filled, ...p }: IconProps & { filled?: boolean }) => (
  <svg
    className={p.className}
    width={p.size ?? 15}
    height={p.size ?? 15}
    viewBox="0 0 24 24"
    fill={filled ? "currentColor" : "none"}
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden
  >
    <path d="M12 17v5" />
    <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
  </svg>
);
export const Ellipsis = (p: IconProps) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="1" />
    <circle cx="19" cy="12" r="1" />
    <circle cx="5" cy="12" r="1" />
  </Svg>
);
export const ChevronDown = (p: IconProps) => (
  <Svg {...p}>
    <path d="m6 9 6 6 6-6" />
  </Svg>
);
export const ChevronRight = (p: IconProps) => (
  <Svg {...p}>
    <path d="m9 18 6-6-6-6" />
  </Svg>
);
export const GitBranch = (p: IconProps) => (
  <Svg {...p}>
    <line x1="6" x2="6" y1="3" y2="15" />
    <circle cx="18" cy="6" r="3" />
    <circle cx="6" cy="18" r="3" />
    <path d="M18 9a9 9 0 0 1-9 9" />
  </Svg>
);
export const X = (p: IconProps) => (
  <Svg {...p}>
    <path d="M18 6 6 18" />
    <path d="m6 6 12 12" />
  </Svg>
);
export const Plus = (p: IconProps) => (
  <Svg {...p}>
    <path d="M5 12h14" />
    <path d="M12 5v14" />
  </Svg>
);
export const Check = (p: IconProps) => (
  <Svg {...p}>
    <path d="M20 6 9 17l-5-5" />
  </Svg>
);
export const ArrowUpDown = (p: IconProps) => (
  <Svg {...p}>
    <path d="m21 16-4 4-4-4" />
    <path d="M17 20V4" />
    <path d="m3 8 4-4 4 4" />
    <path d="M7 4v16" />
  </Svg>
);
export const Search = (p: IconProps) => (
  <Svg {...p}>
    <circle cx="11" cy="11" r="8" />
    <path d="m21 21-4.3-4.3" />
  </Svg>
);

// ── the two originals ──
// Hand-drawn in a 16 viewBox before the set existed; kept verbatim because they
// already render at the same effective stroke as everything above.
export const Folder16 = () => (
  <svg
    className="picon-svg"
    width="15"
    height="15"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.3"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden
  >
    <path d="M1.75 4.25c0-.83.67-1.5 1.5-1.5h2.9l1.6 1.7h5c.83 0 1.5.67 1.5 1.5v6c0 .83-.67 1.5-1.5 1.5H3.25c-.83 0-1.5-.67-1.5-1.5v-7.7Z" />
  </svg>
);
export const Home = () => (
  <svg
    className="picon-svg"
    width="15"
    height="15"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.3"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden
  >
    <path d="M2.5 6.7 8 2.2l5.5 4.5V13a1 1 0 0 1-1 1H9.8v-3.8H6.2V14H3.5a1 1 0 0 1-1-1V6.7Z" />
  </svg>
);

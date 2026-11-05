// The Settings header's search box (the owner's: "a small box that expands
// when clicked into", with a magnifying glass): a combobox whose list is the
// autocomplete. A row names a section, where it lives (group › page) and, when
// the current mode hides it, its tier; choosing one opens that page, scrolls to
// the section and flashes it — the same `goto` a deep link takes, so a hidden
// section is REVEALED for the visit and the mode never changes.
//
// What matches, and in what order, is settingsSearch.ts (pure, checked by
// settingssearch-check.mjs). This file is only the box.
//
// Escape, one step per press (useEscape, so it is the TOP surface while it
// has something to undo and the sheet keeps the next press): an open list
// closes; then the text clears; then the sheet closes as before. The box is
// named `SettingsSearchBox` and not `SettingsSearch` on purpose: on macOS's
// case-insensitive disk `./SettingsSearch` would resolve to settingsSearch.ts.
import { useEffect, useId, useMemo, useRef, useState } from "react";
import * as Icons from "./icons";
import { useEscape } from "./useEscape";
import {
  GROUPS, MODE_LABEL, PAGES, SECTION, SECTIONS, effectiveTier, sectionApplies, sectionWhy,
  type PageId, type VisCtx,
} from "./settingsMap";
import { searchIndex, searchSettings } from "./settingsSearch";

/** The map never changes at runtime: one index for the app's life. */
const INDEX = searchIndex({ sections: SECTIONS, pages: PAGES, groups: GROUPS });

export function SettingsSearchBox({ vis, onOpen, focusToken }: {
  vis: VisCtx;
  /** Open `page` at `section` (SettingsSheet's `goto`). */
  onOpen: (page: PageId, section: string) => void;
  /** Bumped by ⌘F while Settings is the only dialog up (App's chord). */
  focusToken?: number;
}) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [hi, setHi] = useState(0);
  const [focused, setFocused] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const uid = useId();
  const listId = `${uid}-list`;
  const optId = (i: number) => `${uid}-opt-${i}`;

  // What this window can never draw is left out (a host-only section on a
  // remote window, the light/dark pair while the theme is not System). A
  // section a remote window may not READ is listed: its GrantNote says why.
  const hits = useMemo(
    () => searchSettings(INDEX, query, { include: (id) => sectionApplies(SECTION[id], vis) }),
    [query, vis],
  );
  const typed = query.trim() !== "";
  const showList = open && typed;
  const expanded = focused || query !== "";

  // Each Escape undoes one thing: the list, then the text. Active only while
  // there is something to undo, so an empty box hands the press to the sheet.
  useEscape(() => {
    if (showList) setOpen(false);
    else setQuery("");
  }, showList || query !== "");

  // ⌘F. `preventScroll`: the header is not in a scroller, but a focus that
  // scrolls a clipping ancestor is how a header lost 34px once (AGENTS.md).
  const firstToken = useRef(focusToken);
  useEffect(() => {
    if (focusToken === undefined || focusToken === firstToken.current) return;
    const el = inputRef.current;
    if (!el) return;
    el.focus({ preventScroll: true });
    el.select();
    if (el.value.trim()) setOpen(true);
  }, [focusToken]);

  // A press outside the box AND its list closes the list. The list must be
  // excluded too: a handler that only spared the input unmounted the row
  // under the pointer before its click landed (AGENTS.md, the usage panel).
  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", away, true);
    return () => document.removeEventListener("pointerdown", away, true);
  }, [open]);

  const choose = (i: number) => {
    const h = hits[i];
    if (!h) return;
    setOpen(false);
    onOpen(h.page as PageId, h.id);
  };

  const onKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!showList) { if (typed) { setOpen(true); setHi(0); } return; }
      const n = hits.length;
      if (n) setHi((i) => (i + (e.key === "ArrowDown" ? 1 : -1) + n) % n);
      return;
    }
    if (e.key === "Enter" && showList) {
      e.preventDefault();
      choose(hi);
    }
  };

  return (
    <div className={"settings-search" + (expanded ? " wide" : "")} ref={wrapRef} data-testid="settings-search">
      <Icons.Search size={12} className="settings-search-icon" />
      <input
        ref={inputRef}
        className="settings-search-input"
        type="text"
        value={query}
        placeholder={expanded ? "Search settings" : "Search"}
        role="combobox"
        aria-label="Search settings"
        aria-autocomplete="list"
        aria-expanded={showList}
        // Only while the listbox exists: "No settings match" is a note, not a list.
        aria-controls={showList && hits.length > 0 ? listId : undefined}
        // Focus stays in the input, so without this the arrows are silent to
        // a screen reader and Enter opens a row it never announced.
        aria-activedescendant={showList && hits[hi] ? optId(hi) : undefined}
        data-testid="settings-search-input"
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        autoComplete="off"
        onFocus={() => { setFocused(true); if (typed) setOpen(true); }}
        onBlur={() => { setFocused(false); setOpen(false); }}
        onChange={(e) => { setQuery(e.currentTarget.value); setHi(0); setOpen(true); }}
        onKeyDown={onKey}
      />
      {showList && (
        <div className="settings-search-pop" data-testid="settings-search-pop"
          // A press anywhere in the list keeps focus in the input: a blur would
          // close the list under the click.
          onPointerDown={(e) => e.preventDefault()} onMouseDown={(e) => e.preventDefault()}>
          {hits.length === 0 && <div className="settings-search-none" data-testid="settings-search-none">No settings match</div>}
          {hits.length > 0 && (
            <div role="listbox" id={listId} aria-label="Settings">
              {hits.map((h, i) => {
                const sec = SECTION[h.id];
                // The chip says the mode hides it; a section drawn for another
                // reason (changed, a problem, revealed this visit) is on screen.
                const hidden = sectionWhy(sec, vis) === null;
                const tier = effectiveTier(sec, vis);
                return (
                  <div
                    key={h.id}
                    id={optId(i)}
                    role="option"
                    aria-selected={i === hi}
                    className={"settings-search-row" + (i === hi ? " hi" : "")}
                    data-testid="settings-search-row"
                    data-id={h.id}
                    onPointerEnter={() => setHi(i)}
                    onClick={() => choose(i)}
                  >
                    <span className="settings-search-label">{h.label}</span>
                    {hidden && (
                      <span className="settings-search-tier" data-tier={tier} data-testid="settings-search-tier">
                        <span className="settings-search-dot" aria-hidden="true" />{MODE_LABEL[tier]}
                      </span>
                    )}
                    <span className="settings-search-where">{h.where}</span>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

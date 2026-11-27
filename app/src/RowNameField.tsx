// The inline field a nav row turns into while it is renamed (rowname.ts has
// the rules). Module scope, as every input here must be: App() re-renders on
// every poll, and a component defined inside it would remount and lose the
// caret (AGENTS.md).
//
// Uncontrolled. The name on screen starts selected; Enter and blur save,
// Escape cancels through `useEscape` (xterm stops propagation, so a bubble
// listener would miss it, and the field must own the key while it is up).
// The row under it neither selects, nor enters, nor starts a drag from a
// press inside the field.
import { useRef } from "react";
import { useEscape } from "./useEscape";

export function RowNameField({ initial, label, placeholder, testid, className, onSave, onCancel }: {
  /** What the field starts with: the name on screen. */
  initial: string;
  /** Its accessible name ("Name for feat-x"). Never in a usage key. */
  label: string;
  placeholder?: string;
  testid: string;
  className?: string;
  /** The typed text, untrimmed; the caller's rule decides what it means. */
  onSave: (typed: string) => void;
  onCancel: () => void;
}) {
  const done = useRef(false);
  const finish = (f: () => void) => { if (!done.current) { done.current = true; f(); } };
  useEscape(() => finish(onCancel));
  const stop = (e: React.SyntheticEvent) => e.stopPropagation();
  return (
    <input
      className={"title-input row-name-input" + (className ? " " + className : "")}
      data-testid={testid}
      defaultValue={initial}
      autoFocus
      spellCheck={false}
      maxLength={120}
      aria-label={label}
      placeholder={placeholder}
      onFocus={(e) => e.currentTarget.select()}
      onPointerDown={stop}
      onMouseDown={stop}
      onClick={stop}
      onDoubleClick={stop}
      onContextMenu={stop}
      onBlur={(e) => { const v = e.currentTarget.value; finish(() => onSave(v)); }}
      onKeyDown={(e) => {
        // Typing stays in the field: a row's own keys (the menu key, F2,
        // Space on a button row) must not act on it. A ⌘ or Ctrl chord
        // still reaches the app, and the blur it causes saves.
        if (!e.metaKey && !e.ctrlKey) e.stopPropagation();
        if (e.key === "Enter") { e.preventDefault(); const v = e.currentTarget.value; finish(() => onSave(v)); }
      }}
    />
  );
}

// A link in this window never takes the window with it. Every anchor the
// page draws has a handler that opens it (or does nothing) and cancels the
// click; this cancels the click of EVERY anchor, whatever its handler does or
// forgets, before any of them run. It stops nothing else: the handlers still
// run. What WebKit does with no DOM event (its menu's Open Link) is the
// window's own rule (companion.rs `stays`).
export function guardLinks(doc: Pick<Document, "addEventListener">): void {
  const cancel = (e: Event) => {
    const link = e.composedPath().some((n) => (n as Element).nodeName?.toLowerCase() === "a" && (n as Element).hasAttribute?.("href"));
    if (link) e.preventDefault();
  };
  // `click` is the button and Enter; a middle button is `auxclick` alone.
  doc.addEventListener("click", cancel, true);
  doc.addEventListener("auxclick", cancel, true);
}

/** A folder macOS guards with a privacy prompt (AGENTS.md: the first touch
 *  asks, and a denial takes contents while leaving metadata). A project there
 *  works, and asks; the New project and Clone dialogs say so before they
 *  write. Pure: no runtime import (`plans-check.mjs` loads it). */
export function privacyArea(path: string, mac: boolean): "Documents" | "Desktop" | "Downloads" | null {
  if (!mac) return null;
  const m = /^(?:~|\/Users\/[^/]+)\/(documents|desktop|downloads)(?:\/|$)/i.exec(path.trim());
  if (!m) return null;
  return ({ documents: "Documents", desktop: "Desktop", downloads: "Downloads" } as const)[m[1].toLowerCase() as "documents"];
}
export const privacyWords = (area: string): string =>
  `This is in your ${area} folder: macOS asks before worktrees and its agents may read it, and again after each update. A folder such as ~/workspace asks nothing.`;

/** Narrow declaration of the upstream API consumed by Worktrees.
 *
 * Deliberately narrower than the SDK's own types: every option this app does
 * NOT set is omitted, and the three privacy opt-outs are typed as the literal
 * `false` rather than `boolean`, so a future edit that flips one has to change
 * this file too. The payload boundary is the point of the integration; it
 * should not be reachable by a one-character change.
 *
 * Upstream: Orfis `3f8c14b`, `packages/sdk-web` (MIT). See README.md beside
 * this file. The API surface is unchanged from `3222a74` — that revision adds
 * only the license and a banner — so nothing here moved with the re-pin.
 */

/** `onQueued`'s argument. `durable: false` means the report is in memory only
 *  and is lost on quit — the notice must say so, which is the whole reason
 *  upstream made this a typed result instead of a bare callback. */
export interface QueuedResult {
  kind: "queued";
  stored: true;
  durable: boolean;
  reason?: "memory-fallback";
}

export type OrfisTheme = "light" | "dark" | "system";

export interface OrfisOptions {
  key: string;
  apiUrl: string;
  appVersion: string;
  surface: "macos-native";
  /** Worktrees drives the dialog from Settings; the SDK's own launcher is off. */
  launcher: false;
  title: string;
  /** No console/network/error hooks are installed. */
  captureDiagnostics: false;
  /** No `device` block (locale, timezone, viewport, page path, referrer host,
   *  browser hints) and no `elapsedMs` form timing. */
  collectDeviceContext: false;
  /** No reply-email field, so no address can be sent. */
  askForEmail: false;
  theme: OrfisTheme;
  onOpen(): void;
  onClose(): void;
  onQueued(result: QueuedResult): void;
  onSubmitted(id: string): void;
}

export interface OrfisWidget {
  open(): void;
  close(): void;
  destroy(): void;
  setTheme(theme: OrfisTheme): void;
  readonly pending: number;
}

export function init(options: OrfisOptions): OrfisWidget;

// The notification plugin's calls, in one place, each guarded: a window that
// cannot notify (a browser page, a platform without the plugin, a permission
// refused) carries on without one. Deciding WHAT to announce is notify.ts.
// The mock harness installs `window.__mockNotify` and records instead.
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

type MockNotify = { granted: () => boolean; request: () => string; send: (title: string, body: string) => void };
const mock = (): MockNotify | undefined => (window as unknown as { __mockNotify?: MockNotify }).__mockNotify;

/** Has this machine allowed the app's notifications? */
export async function notificationsAllowed(): Promise<boolean | null> {
  const m = mock();
  if (m) return m.granted();
  try { return await isPermissionGranted(); } catch { return null; }
}

/** Ask the system — only from a Settings control (an act), never on load. */
export async function askNotifications(): Promise<boolean | null> {
  const m = mock();
  if (m) return m.request() === "granted";
  try { return (await requestPermission()) === "granted"; } catch { return null; }
}

export function notify(title: string, body: string): void {
  const m = mock();
  if (m) { m.send(title, body); return; }
  try { sendNotification({ title, body }); } catch { /* no plugin here: nothing to say it with */ }
}

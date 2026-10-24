// A section a remote window does not show because its device's grant cannot
// reach the row behind it (Phase 2 §4.4): said, not left blank. Which rows,
// and whether this device may, is ipc.ts's (`notGranted`); serve still
// decides every call.
import { CONSOLE_ROWS, notGranted, remoteHost } from "./ipc";

/** Why `label` is not shown here, as a settings section; nothing when this
 *  window may ask for `cmd`, unless serve already `refused` it (the backstop:
 *  serve decides every call, and its refusal says the same). */
export function GrantNote({ cmd, label, focus, refused = false }: { cmd: string; label: string; focus?: string; refused?: boolean }) {
  if (!notGranted(cmd) && !refused) return null;
  const host = remoteHost() ?? "the host";
  return (
    <section className="setting" data-focus={focus} data-testid="grant-note">
      <label>{label}</label>
      <div className="hint">
        {CONSOLE_ROWS.includes(cmd)
          ? `This is set up at ${host} itself; no paired device may.`
          : `This needs admin on ${host}, and this device was paired without it. Set it up at ${host}, or pair this device again with admin.`}
      </div>
    </section>
  );
}

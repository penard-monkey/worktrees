// "Set by the environment" (settings-reorg §6 item 4): a policy control whose
// value an environment variable of the ENGINE decides looked stuck or wrong,
// with nothing on screen saying why. The getters now report what decided the
// value (`worktrees_core::config::Origin`), and this says it.
//
// The words follow the real order, which is not the same for all three:
// Codex permissions and pi trust are host policy > env > config, so a choice
// made here outranks the variable from then on; agent guidance's `enabled`
// is env > file, so the switch cannot move it while the variable is set.

/** `worktrees_core::config::Origin`, as the getters send it. */
export type Origin = {
  source: "host" | "env" | "config" | "file" | "default";
  env: { name: string; value: string } | null;
};

export function EnvLine({ origin, outranks }: {
  origin: Origin | null | undefined;
  /** "control": a choice here wins over the variable (host policy);
   *  "env": the variable wins over the control. */
  outranks: "control" | "env";
}) {
  const env = origin?.env;
  if (!origin || !env) return null;
  const set = <code>{env.name}={env.value}</code>;
  if (origin.source === "env") {
    return <div className="hint env-line" data-testid="env-origin" data-source="env">
      Set by {set} in the environment the engine was started from. {outranks === "control"
        ? "Choosing here saves this computer's own setting, which wins over it from then on."
        : "It wins over this switch: unset it where the engine starts to change it here."}
    </div>;
  }
  if (origin.source === "host") {
    return <div className="hint env-line" data-testid="env-origin" data-source="host">
      {set} is set in the engine's environment, but this computer's own setting wins over it.
    </div>;
  }
  return <div className="hint env-line" data-testid="env-origin" data-source={origin.source}>
    {set} is set in the engine's environment, but holds a value it does not take, so it decides nothing.
  </div>;
}

// The harness × model picker (pi-harness §2.3/§2.4): one component for the
// new-worktree dialog and the "Switch agent…" sheet, fed by the backend's
// `ModelOption`s (`choice::options_for`).
//
// It says WHY a model cannot be used, rather than hiding it: pi calls a dead
// model host ready and pi's own default may be a provider nobody signed in to,
// so a picker that only listed the usable rows would leave the user wondering
// where the model they configured went.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { NEEDS_MODEL, type Harness } from "./harness";

export type ModelReason = "no_credentials" | "endpoint_unreachable" | "not_served";

export type ModelOption = {
  model: { harness: string; backend: string | null; model: string; label: string | null };
  ready: boolean;
  reason: ModelReason | null;
  source: string;
  meta: { context?: string; max_out?: string; thinking?: boolean; images?: boolean };
};

export const REASON_TEXT: Record<ModelReason, string> = {
  no_credentials: "not signed in",
  endpoint_unreachable: "host not answering",
  not_served: "not loaded on its host",
};

/** The harness's own spelling of a model: `backend/model` for pi. */
export const modelArg = (o: ModelOption) => (o.model.backend ? `${o.model.backend}/${o.model.model}` : o.model.model);

/** A model string the backend will accept as data (`choice::validate_model`). */
export const modelOk = (m: string) => /^[A-Za-z0-9._/:-]{1,200}$/.test(m) && !m.startsWith("-");

/** `harness`'s options, fetched when it changes. `null` while reading. */
export function useAgentModels(harness: Harness, onError: (e: unknown) => void): ModelOption[] | null {
  const [opts, setOpts] = useState<ModelOption[] | null>(null);
  useEffect(() => {
    let alive = true;
    setOpts(null);
    invoke<ModelOption[]>("agent_models", { harness })
      .then((o) => { if (alive) setOpts(o); })
      .catch((e) => { if (alive) { setOpts([]); onError(e); } });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- one read per harness
  }, [harness]);
  return opts;
}

/** What to preselect: the configured default; with none, "" — the CLI's own
 *  default — for a harness where that is safe, and pi's first ready model
 *  where it is not (`NEEDS_MODEL`). */
export function defaultModel(opts: ModelOption[] | null, preferred: string | undefined, harness: Harness): string {
  if (!preferred && !NEEDS_MODEL[harness]) return "";
  if (!opts) return preferred ?? "";
  if (preferred && opts.some((o) => o.ready && modelArg(o) === preferred)) return preferred;
  if (preferred && !opts.some((o) => modelArg(o) === preferred)) return preferred; // free text the user chose
  return opts.find((o) => o.ready) ? modelArg(opts.find((o) => o.ready)!) : "";
}

const OTHER = "\u0000other";

/** A `<select>` grouped by backend, not-ready rows disabled and captioned,
 *  plus an "Other…" row that turns into a text field. `value` "" means the
 *  CLI's own default — offered only where that default is safe (`NEEDS_MODEL`). */
export function ModelPicker({ harness, value, options, onChange, testid, disabled }: {
  harness: Harness;
  value: string;
  options: ModelOption[] | null;
  onChange: (m: string) => void;
  testid?: string;
  disabled?: boolean;
}) {
  const listed = !!options?.some((o) => modelArg(o) === value);
  const [other, setOther] = useState(false);
  const typing = other || (!!value && !!options && !listed);
  const groups = new Map<string, ModelOption[]>();
  for (const o of options ?? []) {
    const k = o.model.backend ?? "";
    groups.set(k, [...(groups.get(k) ?? []), o]);
  }
  const chosen = options?.find((o) => modelArg(o) === value);
  const row = (o: ModelOption) => (
    <option key={modelArg(o)} value={modelArg(o)} disabled={!o.ready}>
      {o.model.label ?? o.model.model}{o.reason ? ` — ${REASON_TEXT[o.reason]}` : ""}
    </option>
  );
  return (
    <div className="model-picker">
      <select className="np-input" data-testid={testid} disabled={disabled || options === null}
        aria-label={`${harness} model`}
        value={typing ? OTHER : value}
        onChange={(e) => {
          const v = e.currentTarget.value;
          if (v === OTHER) { setOther(true); return; }
          setOther(false);
          onChange(v);
        }}>
        {options === null && <option value={value}>reading models…</option>}
        {options !== null && (NEEDS_MODEL[harness]
          ? <option value="" disabled>{options.some((o) => o.ready) ? "choose a model" : "no usable model — see Settings → pi"}</option>
          : <option value="">the CLI's default</option>)}
        {[...groups.entries()].map(([backend, rows]) => backend
          ? <optgroup key={backend} label={backend}>{rows.map(row)}</optgroup>
          : rows.map(row))}
        {options !== null && <option value={OTHER}>Other…</option>}
      </select>
      {typing && (
        <input className="np-input" data-testid={testid ? `${testid}-other` : undefined} autoFocus
          placeholder={harness === "pi" ? "backend/model-id" : "model id"} spellCheck={false}
          autoCapitalize="off" autoCorrect="off" value={value}
          onChange={(e) => onChange(e.currentTarget.value.trim())} />
      )}
      {typing && value && !modelOk(value) && (
        <span className="np-hint warn">letters, digits and . _ / : - only</span>
      )}
      {chosen && !chosen.ready && chosen.reason && (
        <span className="np-hint warn">{value}: {REASON_TEXT[chosen.reason]}</span>
      )}
    </div>
  );
}

/** `pi_status`: which pi and node a pane gets, and the trust posture. */
export type PiStatus = {
  preflight: {
    pi_path: string | null; pi_version: string | null;
    node_path: string | null; node_version: string | null;
    node_floor: string | null; problem: string | null;
  };
  trust: "never" | "ask";
  allowed: string[];
  repo_root: string | null;
  repo_allowed: boolean;
};

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { DiffView, type FileDiffDto } from "./DiffView";
import { useEscape } from "./useEscape";

/** `guidance::Status` (core), as `agent_guidance_status` returns it. */
export type GuidanceDelivery =
  | { state: "on"; flags: string[] }
  | { state: "off" }
  | { state: "skipped"; reason: string }
  /** Codex before any launch has asked it — status never probes, because the
   *  probe starts Codex's MCP servers. */
  | { state: "unchecked" };
export type GuidanceHarness = { id: string; label: string; installed: boolean } & GuidanceDelivery;
/** `guidance::SkillEdit`: the user's own copy of the skill, as it is on disk. */
export type GuidanceSkillEdit = {
  text: string;
  /** The shipped default the edit was based on — null when that record is
   *  missing or damaged, which reads as stale. */
  base: { version: number; hash: string; text: string } | null;
  /** The default changed since the edit was based on it. */
  stale: boolean;
  /** Why agents get the default instead, when they do. */
  invalid: string | null;
};
export type GuidanceStatus = {
  /** Bumped when what agents are told changes enough to ask again (the offer's fingerprint). */
  version: number;
  settings: { enabled: boolean; guard: boolean };
  /** A `worktrees` CLI that HAS the guard is on PATH. Without one, Claude gets
   *  the plain plugin even with the guard switched on. */
  guard_available: boolean;
  settings_path: string;
  dir: string | null;
  error: string | null;
  harnesses: GuidanceHarness[];
  /** What agents get: the user's edit when usable, else the default. */
  skill: string;
  /** The skill this build ships. */
  skill_default: string;
  /** Hash of `skill_default` — the `agent-guidance-changed` offer's fingerprint. */
  skill_hash: string;
  skill_edit: GuidanceSkillEdit | null;
  /** Where the edit lives (`~/.config/worktrees/guidance/SKILL.md`). */
  edit_path: string;
  rules: string;
};

function deliveryLine(h: GuidanceHarness): string {
  if (!h.installed) return "not installed";
  switch (h.state) {
    case "on":
      return h.id === "claude" ? "gets the worktrees plugin (skill)" : h.id === "pi" ? "gets the skill and the rule" : "gets the rule";
    case "off":
      return "off";
    case "skipped":
      return `not given it: ${h.reason}`;
    case "unchecked":
      return "decided the next time Worktrees launches it (it checks for your own developer_instructions first)";
  }
}

/** One text against another, through the Files tab's renderer. The backend
 *  runs `git diff --no-index` at full context, so the patch is the same shape
 *  `DiffView` already parses. Responses carry a sequence number: a draft
 *  typed quickly fires several, and only the newest may land. */
function GuidanceDiff({ from, to, fromLabel, toLabel, onReport }: {
  from: string; to: string; fromLabel: string; toLabel: string; onReport: (t: string) => void;
}) {
  const [patch, setPatch] = useState<string | null>(null);
  // A failed diff says so in the box: left at "diffing…" it reads as slow,
  // forever, rather than broken.
  const [err, setErr] = useState<string | null>(null);
  const seq = useRef(0);
  useEffect(() => {
    const n = ++seq.current;
    const t = setTimeout(() => {
      invoke<string>("agent_guidance_diff", { old: from, new: to })
        .then((p) => { if (n === seq.current) { setPatch(p); setErr(null); } })
        .catch((e) => {
          if (n !== seq.current) return;
          setErr(String(e));
          onReport(`agent_guidance_diff: ${String(e)}`);
        });
    }, 200);
    return () => clearTimeout(t);
  }, [from, to]);
  const dto: FileDiffDto | null = patch === null ? null
    : { against: "base", base_label: fromLabel, patch, untracked: false, binary: false, truncated: false };
  return <div className="guidance-diff" data-testid="guidance-diff">
    <div className="guidance-diff-head"><span className="del">− {fromLabel}</span><span className="add">+ {toLabel}</span></div>
    {err ? <div className="tree-note err-note" role="alert">Could not compare: {err}</div>
      : !dto ? <div className="tree-note">diffing…</div> : <DiffView diff={dto} content="" lang="" wrap />}
  </div>;
}

type Compare = "default" | "yours-new" | "yours";

/** The banner for an edit whose default moved under it (an update shipped a
 *  new skill). Agents keep getting the user's text until they choose — the
 *  three ways out are the three buttons, and "merge" only fills the editor. */
function SkillChanged({ status, edit, busy, editing, onKeep, onTake, onMerge, offerPending, onSilenceOffer, onReport }: {
  status: GuidanceStatus;
  edit: GuidanceSkillEdit;
  busy: boolean;
  /** A draft is open below: the band stays (it is the offer's deep-link
   *  target) but hands the choosing to the editor. */
  editing: boolean;
  onKeep: () => void;
  onTake: () => void;
  onMerge: () => void;
  offerPending: boolean;
  onSilenceOffer?: () => void;
  onReport: (t: string) => void;
}) {
  const base = edit.base;
  // With no record of the base there is one honest comparison, not three.
  const [mode, setMode] = useState<Compare>(base ? "default" : "yours-new");
  const modes: { id: Compare; label: string }[] = base
    ? [{ id: "default", label: "What the default changed" }, { id: "yours-new", label: "Yours vs the new default" }, { id: "yours", label: "What you changed" }]
    : [{ id: "yours-new", label: "Yours vs the new default" }];
  const pair = mode === "default" && base ? { from: base.text, to: status.skill_default, fl: `the default you edited (v${base.version})`, tl: "the new default" }
    : mode === "yours" && base ? { from: base.text, to: edit.text, fl: `the default you edited (v${base.version})`, tl: "your edit" }
    : { from: edit.text, to: status.skill_default, fl: "your edit", tl: "the new default" };
  return <div className="guidance-changed" data-focus="agent-guidance-changed" data-testid="guidance-changed">
    <div className="guidance-changed-title">The default skill changed in this update</div>
    <div className="hint">
      {base ? `You edited the skill as it was in guidance v${base.version}, and the default has changed since. `
        : "You edited the skill, and the record of which default you started from is missing, so it cannot be told apart from this one. "}
      {editing ? "You are editing below; saving makes it your edit, based on the new default." : "Agents keep getting your edit until you choose."}
    </div>
    {!editing && <>
    {modes.length > 1 && <div className="seg seg-plain guidance-modes" role="radiogroup" aria-label="Compare">
      {modes.map((m) => <button key={m.id} role="radio" aria-checked={mode === m.id} className={mode === m.id ? "on" : ""}
        onClick={() => setMode(m.id)}>{m.label}</button>)}
    </div>}
    <GuidanceDiff from={pair.from} to={pair.to} fromLabel={pair.fl} toLabel={pair.tl} onReport={onReport} />
    <div className="ver-actions">
      <button className="ctrl sm" disabled={busy} onClick={onKeep} title="Agents keep your text; it is now based on this default, so the next change asks again">Keep mine</button>
      <button className="ctrl sm" disabled={busy} onClick={onTake} title="Discard your edit; agents get the new default">Use the new default</button>
      {base && <button className="ctrl sm" disabled={busy} onClick={onMerge} title="Your changes replayed onto the new default, in the editor — nothing is saved until you press Save">Merge into the editor…</button>}
      {offerPending && onSilenceOffer && <button className="mcp-dismiss" onClick={onSilenceOffer}>Stop suggesting this</button>}
    </div>
    </>}
  </div>;
}

/** The draft outlives the component: Settings unmounts the section when you
 *  switch category or close the sheet, and a half-written edit of a 10 KB
 *  skill is not something to lose to a stray click. */
let keptDraft: string | null = null;

/** How long "Reset to default" stays armed. */
const ARM_MS = 4000;

/** Settings → Agent guidance (agent-guidance proposal §4.5, decisions Q2/Q8;
 *  the editable skill, §11). Machine-level: nothing here is about the project
 *  in focus. */
export function GuidanceSection({ status, onStatus, onReport, offerPending = false, onSilenceOffer, changeOfferPending = false, onSilenceChangeOffer, "data-focus": focusId }: {
  /** App's startup probe; null = not read yet or failed. */
  status: GuidanceStatus | null;
  /** Every status read or written here goes back to App, so the offer follows it. */
  onStatus: (s: GuidanceStatus) => void;
  onReport: (text: string) => void;
  offerPending?: boolean;
  onSilenceOffer?: () => void;
  /** The `agent-guidance-changed` offer (offers.ts) is pending. */
  changeOfferPending?: boolean;
  onSilenceChangeOffer?: () => void;
  "data-focus"?: string;
}) {
  const [busy, setBusy] = useState(false);
  const [draft, setDraftState] = useState<string | null>(keptDraft);
  const setDraft = (d: string | null) => { keptDraft = d; setDraftState(d); };
  const [saveErr, setSaveErr] = useState<string | null>(null);
  const [mergeNote, setMergeNote] = useState<string | null>(null);
  const [showDraftDiff, setShowDraftDiff] = useState(false);
  const [armed, setArmed] = useState(false);
  const [open, setOpen] = useState(keptDraft !== null);
  // Re-read on open: cheap (no Codex probe), and a Codex launch since app
  // start may have settled Codex's line.
  useEffect(() => {
    invoke<GuidanceStatus>("agent_guidance_status").then(onStatus).catch((e) => onReport(`agent_guidance_status: ${String(e)}`));
  }, []);
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(false), ARM_MS);
    return () => clearTimeout(t);
  }, [armed]);
  const dirty = draft !== null && draft !== status?.skill;
  // Escape would close the sheet; with unsaved text it does nothing instead —
  // the sheet's own entry is below this one, so the key stops here.
  useEscape(() => {}, dirty);

  const set = async (patch: Partial<GuidanceStatus["settings"]>) => {
    if (!status) return;
    setBusy(true);
    try {
      const next = { ...status.settings, ...patch };
      onStatus(await invoke<GuidanceStatus>("set_agent_guidance", { enabled: next.enabled, guard: next.guard }));
    } catch (e) { onReport(`set_agent_guidance: ${String(e)}`); }
    finally { setBusy(false); }
  };
  /** Save `text` as the skill, or reset with null. Errors that are the user's
   *  to fix (a broken frontmatter, a merge marker) show beside the editor. */
  const saveSkill = async (text: string | null): Promise<boolean> => {
    setBusy(true);
    setSaveErr(null);
    try {
      onStatus(await invoke<GuidanceStatus>("set_agent_guidance_skill", { text }));
      return true;
    } catch (e) {
      setSaveErr(String(e));
      return false;
    } finally { setBusy(false); }
  };
  const merge = async () => {
    setBusy(true);
    try {
      const m = await invoke<{ text: string; conflicts: number }>("agent_guidance_merge");
      setDraft(m.text);
      setOpen(true);
      setShowDraftDiff(false);
      setMergeNote(m.conflicts === 0
        ? "Merged cleanly: your changes on top of the new default. Read it, then Save."
        : `Merged with ${m.conflicts} conflict${m.conflicts === 1 ? "" : "s"}: look for the <<<<<<< your edit markers, keep one side of each, then Save.`);
    } catch (e) { onReport(`agent_guidance_merge: ${String(e)}`); }
    finally { setBusy(false); }
  };

  const edit = status?.skill_edit ?? null;
  const editing = draft !== null;
  const usable = !!edit && !edit.invalid;
  return <section className="setting" data-focus={focusId} data-testid="agent-guidance">
    <label>Agent guidance</label>
    <div className="hint">In a worktrees-managed repo, agents launched from Worktrees are told to do their branch work in a place, and get a <code>worktrees</code> skill for the rest: handing work to another agent, messaging between places, finishing and releasing. The text ships with Worktrees and you can edit the skill; nothing is written to any agent's own settings.</div>
    {!status ? <div className="hint">Checking…</div> : <>
      <label className="tier-toggle setting-check"><input type="checkbox" checked={status.settings.enabled} disabled={busy}
        onChange={(e) => set({ enabled: e.currentTarget.checked })} />Give agents the guidance at launch</label>
      <label className="tier-toggle setting-check"><input type="checkbox" checked={status.settings.guard} disabled={busy || !status.settings.enabled}
        onChange={(e) => set({ guard: e.currentTarget.checked })} />Stop Claude from <code>git worktree add</code> and new branches in (main)</label>
      <div className="hint">The guard refuses those two commands and tells Claude to use a place. An agent moving its own place to another branch is never refused. Switching it off applies to the next command. Off by default.</div>
      {status.settings.guard && !status.guard_available && <div className="hint" data-testid="guard-unavailable">
        The guard needs the Worktrees CLI, and none on your PATH has it (missing, or older than this app). Until you install or update it under Updates, Claude gets the skill without the guard.</div>}
      <div className="guidance-rows">
        {status.harnesses.map((h) => (
          <div className="hint" key={h.id} data-harness={h.id}><b>{h.label}:</b> {status.settings.enabled ? deliveryLine(h) : "off"}</div>
        ))}
      </div>
      {status.error && <div className="hint">The guidance files could not be written: {status.error}</div>}
      <div className="hint">Applies to agents launched from now on; a session already running keeps what it started with. Every connected session also reads the rule through the Worktrees MCP server.</div>
      {edit?.invalid && <div className="guidance-changed" data-testid="guidance-invalid">
        <div className="guidance-changed-title">Your edited skill is not being used</div>
        <div className="hint">{edit.invalid[0].toUpperCase() + edit.invalid.slice(1)}. Agents get the default until you fix it or reset. The file is {status.edit_path}.</div>
        <div className="ver-actions">
          <button className="ctrl sm" disabled={busy || editing} onClick={() => { setDraft(edit.text); setOpen(true); }}>Fix it…</button>
          <button className="ctrl sm" disabled={busy} onClick={() => saveSkill(null)}>Reset to default</button>
        </div>
      </div>}
      {usable && edit.stale && <SkillChanged status={status} edit={edit} busy={busy} editing={editing}
        onKeep={() => saveSkill(edit.text)} onTake={() => saveSkill(null)} onMerge={merge}
        offerPending={changeOfferPending} onSilenceOffer={onSilenceChangeOffer} onReport={onReport} />}
      <details className="guidance-text" open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
        <summary>What agents are told</summary>
        <div className="hint">The rule (pi and Codex get it as a prompt; every session sees it through the MCP server). It is fixed: it is also the MCP server's one-line instructions.</div>
        <pre className="update-log">{status.rules}</pre>
        <div className="guidance-skill-head">
          <div className="hint">The skill (Claude and pi){usable ? <> — <b data-testid="skill-source">your edit</b>{edit.base ? `, based on guidance v${edit.base.version}` : ""}</> : <> — <span data-testid="skill-source">the default</span></>}:</div>
          {!editing && <div className="ver-actions">
            <button className="ctrl sm" disabled={busy} onClick={() => { setDraft(usable ? edit.text : status.skill_default); setSaveErr(null); setMergeNote(null); }}>Edit…</button>
            {usable && <button className="ctrl sm" disabled={busy} onClick={() => { if (!armed) { setArmed(true); return; } setArmed(false); saveSkill(null); }}>
              {armed ? "Discard your edit?" : "Reset to default"}</button>}
          </div>}
        </div>
        {!editing ? <pre className="update-log">{status.skill}</pre> : <div className="guidance-editor">
          {mergeNote && <div className="hint" data-testid="merge-note">{mergeNote}</div>}
          <textarea value={draft} spellCheck={false} aria-label="The worktrees skill" rows={18}
            onChange={(e) => setDraft(e.currentTarget.value)} />
          {saveErr && <div className="hint guidance-err" role="alert">{saveErr}</div>}
          <div className="ver-actions">
            <button className="ctrl sm" disabled={busy || !dirty} onClick={async () => { if (await saveSkill(draft)) { setDraft(null); setMergeNote(null); } }}>Save</button>
            <button className="ctrl sm" disabled={busy} onClick={() => { setDraft(null); setSaveErr(null); setMergeNote(null); }}>{dirty ? "Discard changes" : "Close"}</button>
            <button className="ctrl sm" onClick={() => setShowDraftDiff(!showDraftDiff)}>{showDraftDiff ? "Hide changes" : "Compare with the default"}</button>
          </div>
          {showDraftDiff && <GuidanceDiff from={status.skill_default} to={draft} fromLabel="the default" toLabel="this text" onReport={onReport} />}
          <div className="hint">Saved to {status.edit_path}, with a copy of the default it was based on, so an update that changes the default can show you what moved. Saving the default unchanged is the same as Reset.</div>
        </div>}
      </details>
      <div className="hint">Settings file: {status.settings_path}</div>
      {offerPending && onSilenceOffer && <div className="ver-actions"><button className="mcp-dismiss" onClick={onSilenceOffer}>Stop suggesting this</button></div>}
    </>}
  </section>;
}

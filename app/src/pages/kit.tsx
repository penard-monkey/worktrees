// What every Settings page is built from (settings-reorg §3–§4): the sheet's
// context, the `Sec` wrapper that applies the mode rules to one section, and
// the small pieces a page repeats — its heading with the scope sentence, the
// "N more settings" footer, the pointer to a section that moved.
//
// A page is a component at MODULE scope that reads `useSheet()`; nothing here
// is defined inside another component (AGENTS.md: it would remount on every
// render and lose focus mid-click).
import { createContext, useContext, useEffect, type ReactNode } from "react";
import { notGranted } from "../ipc";
import { GrantLine, GrantNote } from "../GrantNote";
import type { Settings, UpdateInfo, CodexPermissions } from "../settings";
import type { McpStatus } from "../McpPanel";
import type { CodexMcpStatus } from "../CodexMcpPanel";
import type { PiMcpStatus } from "../PiMcpPanel";
import type { KimiMcpStatus, KimiStatus } from "../KimiPanel";
import type { UserSkill } from "../AgentSetup";
import type { GuidanceStatus } from "../GuidancePanel";
import type { CrossProjectStatus } from "../CrossProjectPanel";
import type { PlanningStatus } from "../PlanningPanel";
import type { QuotaSettings } from "../QuotaPanel";
import type { SlotsSettings } from "../SlotsPanel";
import type { LimitsSettings } from "../LimitsPanel";
import type { PresetStatus } from "../presets";
import type { PiStatus } from "../ModelPicker";
import type { ResetScope } from "../settingsMap";
import type { UpdateRuns } from "./updates";
import {
  MODE_LABEL, PAGE, SECTION, effectiveTier, pageFooter, scopeChip, scopeSentence,
  type Mode, type PageId, type SettingsAt, type VisCtx, type Why,
} from "../settingsMap";

/** What App hands the sheet. Unchanged by the regroup: pages read it whole. */
export type SheetProps = {
  open: boolean;
  /** Where the sheet was asked to open, when the caller had somewhere in mind —
   *  an offer's deep link (`offers.ts`), an old category id included
   *  (`LEGACY_CATS`). `null` is the ordinary ⌘, open. */
  at: SettingsAt | null;
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onClose: () => void;
  update: UpdateInfo | null;
  cliStale: boolean;
  cliMissing: boolean;
  appStale: boolean;
  onCheckUpdate: (keepLatest?: boolean, force?: boolean) => Promise<unknown> | void;
  /** The page on screen, whichever way it got there (opened at it, or clicked
   *  to). App uses it to retire the update bubble once Updates has been seen. */
  onCatShown?: (cat: PageId) => void;
  onShowNotes: () => void;
  /** Settings → Layout → Reset: App writes the layout defaults through the
   *  same commit a drag makes (arrange.ts) and hands back its undo. */
  onResetLayout: () => (() => void) | null;
  /** Settings → Data & privacy → Reset: App writes the scope's patch through
   *  the ordinary settings path and resets the React state that mirrors it. */
  onReset: (patch: Partial<Settings>, scope: ResetScope) => void;
  /** The project in focus — AI profiles can be bound per repo. "" is fine. */
  repo: string;
  onReport: (msg: string) => void;
  /** App's `fail`: a refused host-policy change is an error, not a notice. */
  onFail: (e: unknown) => void;
  /** Settings → Projects: close the sheet and open a project's own sheet. */
  onOpenProject?: (root: string) => void;
  /** Bumped by ⌘F while Settings is the only dialog up: focus the header's
   *  search box (App's chord handler). */
  findToken?: number;
  mcpStatus: McpStatus | null;
  onMcpChanged: (s: McpStatus) => void;
  mcpOfferPending: boolean;
  onSilenceMcpOffer: () => void;
  onCodexMcpChanged: (s: CodexMcpStatus) => void;
  codexMcpOfferPending: boolean;
  onSilenceCodexMcpOffer: () => void;
  onPiMcpChanged: (s: PiMcpStatus) => void;
  piMcpOfferPending: boolean;
  onSilencePiMcpOffer: () => void;
  onKimiMcpChanged: (s: KimiMcpStatus) => void;
  kimiMcpOfferPending: boolean;
  onSilenceKimiMcpOffer: () => void;
  userSkills: UserSkill[] | null;
  onUserSkillsChanged: (s: UserSkill[]) => void;
  skillsOfferPending: boolean;
  onSilenceSkillsOffer: () => void;
  guidance: GuidanceStatus | null;
  onGuidanceChanged: (s: GuidanceStatus) => void;
  guidanceOfferPending: boolean;
  onSilenceGuidanceOffer: () => void;
  guidanceChangeOfferPending: boolean;
  onSilenceGuidanceChangeOffer: () => void;
  crossProject: CrossProjectStatus | null;
  onCrossProjectChanged: (s: CrossProjectStatus) => void;
  crossProjectOfferPending: boolean;
  onSilenceCrossProjectOffer: () => void;
  planning: PlanningStatus | null;
  onPlanningChanged: (s: PlanningStatus) => void;
  planningOfferPending: boolean;
  onSilencePlanningOffer: () => void;
};

/** The host's values the sheet reads when it opens and on every page change
 *  (cheap getters, all of them), for the mode rules and the pointer lines.
 *  Each is absent until read, and stays absent where this window may not ask. */
export type HostSnap = {
  codexPermissions?: CodexPermissions;
  fetchInterval?: number;
  termHistory?: { persist_scrollback: boolean; per_tab_history: boolean };
  quota?: QuotaSettings;
  slots?: SlotsSettings;
  limits?: LimitsSettings;
  preset?: PresetStatus;
  pi?: PiStatus;
  kimi?: KimiStatus;
  profiles?: { count: number; hasDefault: boolean };
};

export type SheetApi = SheetProps & {
  mode: Mode;
  setMode: (m: Mode) => void;
  /** Move within the sheet, and say which section was meant. */
  goto: (cat: PageId, focus?: string) => void;
  snap: HostSnap;
  why: (id: string) => Why | null;
  /** A page that learns of a problem in one of its sections (rule 3). */
  reportProblem: (id: string, on: boolean) => void;
  /** The paired host's name on a window that drives one; null on the host. */
  host: string | null;
  vis: VisCtx;
  /** Every update the sheet can be running (pages/updates.tsx): held by the
   *  sheet so a page change cannot re-enable a button mid-install. */
  runs: UpdateRuns;
  /** The focus the sheet was sent to (a section, or a band in one). */
  focus: string | null;
};

export const SheetContext = createContext<SheetApi | null>(null);

export function useSheet(): SheetApi {
  const api = useContext(SheetContext);
  if (!api) throw new Error("a Settings page rendered outside the sheet");
  return api;
}

const WHY_WORDS: Record<Exclude<Why, "mode">, string> = {
  problem: "shown because it needs attention",
  changed: "shown because it is not at its default",
  revealed: "shown because a link pointed here",
};

/** One section of a page, drawn or not as the mode says (`settingsMap.ts`):
 *  - hidden: nothing at all (rule 1: what it holds still applies);
 *  - drawn above the mode: with the tier's name and why (rules 2 and 3, and a
 *    deep link's reveal, which also offers the switch);
 *  - a remote window that may not READ it: the GrantNote instead;
 *  - one that may read but not WRITE it: the controls disabled, and why.
 *  `problem`: the page knows of one the sheet cannot (rule 3). */
export function Sec({ id, problem, children }: { id: string; problem?: boolean; children: ReactNode }) {
  const api = useSheet();
  const { reportProblem } = api;
  // Before any early return: a hidden section still reports, or rule 3 could
  // never draw it.
  useEffect(() => {
    if (problem === undefined) return;
    reportProblem(id, problem);
    return () => reportProblem(id, false);
  }, [id, problem, reportProblem]);
  const def = SECTION[id];
  const why = api.why(id);
  if (!def || !why) return null;
  if (def.read && notGranted(def.read)) {
    return <div className="sec" data-focus={id} data-sec={id}><GrantNote cmd={def.read} label={def.label} /></div>;
  }
  const denied = def.write ? notGranted(def.write) : null;
  const tier = effectiveTier(def, api.vis);
  const own = PAGE[def.page].scope;
  const chip = def.scope && def.scope !== own ? scopeChip(def.scope, api.host) : null;
  const above = why !== "mode";
  return (
    <fieldset className="sec" data-focus={id} data-sec={id} data-tier={tier} disabled={!!denied}>
      <span className="sec-dot" aria-hidden="true" title={`${MODE_LABEL[tier]} setting`} />
      {(chip || above) && (
        <div className="sec-eyebrow">
          {chip && <span className="sec-chip" data-testid={`scope-${id}`}>{chip}</span>}
          {above && <span className="sec-chip sec-why" data-testid={`why-${id}`}>{MODE_LABEL[tier]} setting · {WHY_WORDS[why]}</span>}
        </div>
      )}
      {children}
      {denied && def.write && <GrantLine cmd={def.write} />}
      {why === "revealed" && (
        <div className="sec-reveal">
          <span>This is an {MODE_LABEL[tier]} setting; {MODE_LABEL[api.mode]} does not list it.</span>
          <button type="button" className="mcp-dismiss" data-testid={`reveal-switch-${id}`} onClick={() => api.setMode(tier)}>
            Switch to {MODE_LABEL[tier]}
          </button>
        </div>
      )}
    </fieldset>
  );
}

/** A page's title and the one sentence that says whose its settings are. */
export function PageHead({ page }: { page: PageId }) {
  const api = useSheet();
  const p = PAGE[page];
  const line = scopeSentence(p.scope, api.host);
  return (
    <div className="page-head">
      <h2 className="page-title">{p.label}</h2>
      {line && <div className="page-scope" data-testid="page-scope">{line}</div>}
    </div>
  );
}

/** "N more settings in Advanced ›" (§4.3): switches the mode in place. */
export function PageFoot({ page }: { page: PageId }) {
  const api = useSheet();
  const more = pageFooter(page, api.vis);
  if (!more) return null;
  return (
    <div className="page-foot">
      <button type="button" className="page-more" data-testid="page-more" onClick={() => api.setMode(more.next)}>
        {more.count} more setting{more.count === 1 ? "" : "s"} in {MODE_LABEL[more.next]} ›
      </button>
    </div>
  );
}

/** Where a section went (§3.2): a harness page keeps one line with the
 *  current value, because whoever comes here looking must still find it. */
export function Pointer({ to, label, children }: { to: string; label: string; children: ReactNode }) {
  const api = useSheet();
  const page = SECTION[to]?.page;
  return (
    <section className="setting pointer">
      <label>{label}</label>
      <div className="pointer-line">
        <span className="hint">{children}</span>
        {page && (
          <button type="button" className="mcp-dismiss" data-testid={`pointer-${to}`} onClick={() => api.goto(page, to)}>
            Change under {PAGE[page].label} ›
          </button>
        )}
      </div>
    </section>
  );
}

/** A page: heading, sections, footer. */
export function Page({ page, children }: { page: PageId; children: ReactNode }) {
  return (
    <>
      <PageHead page={page} />
      {children}
      <PageFoot page={page} />
    </>
  );
}

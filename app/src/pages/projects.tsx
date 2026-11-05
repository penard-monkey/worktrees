// Settings → Projects (settings-reorg §3.2, §9 Q6 by the plan's default): the
// planning default and each registered project's level, as Settings → Planning
// had them, plus — per project — its AI profile and whether it is private
// (Advanced), and a link to its own sheet, which stays the editor for
// `.worktrees.toml`, relink, provisioning and agent setup. A LIST with links,
// not a second editor: privacy is changed under Agent policy, a profile in the
// project's sheet.
import { useEffect, useState } from "react";
import { invoke, notGranted } from "../ipc";
import { PlanningSection } from "../PlanningPanel";
import type { ProfilesInfo } from "../ProfilesPanel";
import { Page, Sec, useSheet } from "./kit";

/** Each project's profile, by root: `profiles_info` answers for one repo at
 *  a time, so one read per project, and only while the column shows. */
function useProjectProfiles(roots: string[], on: boolean): Record<string, string | null> {
  const [names, setNames] = useState<Record<string, string | null>>({});
  const key = roots.join("\n");
  useEffect(() => {
    if (!on || notGranted("profiles_info")) return;
    let alive = true;
    void Promise.all(roots.map(async (root) => {
      try {
        const i = await invoke<ProfilesInfo | null>("profiles_info", { repo: root });
        const id = i?.assigned_id ?? null;
        return [root, id ? i?.profiles.find((p) => p.id === id)?.name ?? id : null] as const;
      } catch { return [root, null] as const; }
    })).then((rows) => { if (alive) setNames(Object.fromEntries(rows)); });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the roots, not the array's identity
  }, [key, on]);
  return names;
}

export function ProjectsPage() {
  const {
    planning, onPlanningChanged, onReport, planningOfferPending, onSilencePlanningOffer,
    crossProject, mode, onOpenProject,
  } = useSheet();
  const more = mode !== "beginner";
  const roots = planning?.projects.map((p) => p.root) ?? [];
  const profiles = useProjectProfiles(roots, more);
  const isPrivate = (root: string) => crossProject?.projects.find((p) => p.root === root)?.private ?? false;
  return <Page page="projects">
    <Sec id="planning">
      <PlanningSection status={planning} onStatus={onPlanningChanged} onReport={onReport}
        offerPending={planningOfferPending} onSilenceOffer={onSilencePlanningOffer}
        rowExtra={(root) => (
          <div className="project-extra" data-testid="project-extra">
            {more && <span className="hint">Profile: <b>{profiles[root] ?? "none"}</b></span>}
            {more && isPrivate(root) && <span className="hint">· <b>private</b></span>}
            {onOpenProject && (
              <button type="button" className="mcp-dismiss" data-testid="project-settings-link" onClick={() => onOpenProject(root)}>
                Project settings…
              </button>
            )}
          </div>
        )} />
    </Sec>
    <div className="hint page-note">Project settings… opens the project's own sheet. What it writes to <code>.worktrees.toml</code> is the project's own file: every clone gets it.</div>
  </Page>;
}

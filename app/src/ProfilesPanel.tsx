import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

// AI profiles: the rules / skills / MCP / model a worktrees-launched `claude`
// runs with, instead of the user's global ~/.claude setup.
//
// MODULE SCOPE, with props, deliberately: a component defined inside App() gets
// a new identity every render, and this one owns a <textarea> the user types
// prose into. Nested, it would drop focus on every keystroke.
//
// State lives in core (`~/.config/worktrees/profiles.json`), shared with the
// CLI — this panel never keeps its own copy, it re-reads after every write.

export type Profile = {
  id: string;
  name: string;
  rules?: string;
  skills?: string[];
  inherit_global_skills?: boolean;
  mcp_servers?: Record<string, unknown>;
  inherit_global_mcp?: boolean;
  worktrees_mcp?: boolean;
  model?: string | null;
  settings?: unknown;
  updated_epoch?: number;
  // Decorated by the backend, not stored:
  ever_launched?: boolean;
  dir?: string | null;
};

export type ProfilesInfo = {
  profiles: Profile[];
  default_id?: string | null;
  assigned_id?: string | null;
  effective_id?: string | null;
  repo_has_unprofiled_history?: boolean;
  env_override?: string | null;
};

export type SkillEntry = {
  name: string;
  description?: string;
  capabilities?: string[];
  source?: { kind: "local"; path: string } | { kind: "git"; url: string; sha: string };
};

type GitPreview = {
  url: string;
  rev: string;
  sha: string;
  skills: { name: string; description: string; capabilities: string[]; skill_md: string }[];
};

const short = (s: string, n = 8) => (s.length > n ? s.slice(0, n) : s);

export default function ProfilesPanel({ repo, onReport }: { repo: string; onReport: (m: string) => void }) {
  const [info, setInfo] = useState<ProfilesInfo | null>(null);
  const [skills, setSkills] = useState<SkillEntry[]>([]);
  const [sel, setSel] = useState<string>("");
  const [draft, setDraft] = useState<Profile | null>(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [armed, setArmed] = useState("");
  const [gitUrl, setGitUrl] = useState("");
  const [preview, setPreview] = useState<GitPreview | null>(null);

  // Every write is followed by a re-read: core owns this state and the CLI can
  // change it under us.
  const reload = useCallback(async () => {
    try {
      const i = await invoke<ProfilesInfo | null>("profiles_info", { repo });
      const s = await invoke<SkillEntry[] | null>("skills_list");
      setInfo(i ?? { profiles: [] });
      setSkills(s ?? []);
      return i;
    } catch (e) {
      setErr(String(e));
      return null;
    }
  }, [repo]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Never swallow: sheets have no fail(), so the idiom is a local error area
  // plus the app log.
  const fail = (e: unknown, what: string) => {
    const msg = `${what}: ${e}`;
    setErr(msg);
    void invoke("log_event", { level: "error", msg }).catch(() => {});
  };

  const pick = (p: Profile) => {
    setSel(p.id);
    setDraft({ ...p });
    setErr("");
  };

  const save = async (p: Profile) => {
    setBusy(true);
    try {
      // `ever_launched`/`dir` are decoration, not stored fields — sending them
      // back would have serde reject the profile.
      const { ever_launched: _e, dir: _d, ...body } = p;
      await invoke("save_profile", { profile: body });
      await reload();
      onReport(`Saved profile “${p.name}”.`);
    } catch (e) {
      fail(e, "save profile");
    } finally {
      setBusy(false);
    }
  };

  const create = async () => {
    setBusy(true);
    try {
      const name = "New profile";
      const id = await invoke<string>("new_profile_id", { name });
      const p: Profile = { id, name, rules: "", skills: [] };
      await invoke("save_profile", { profile: p });
      await reload();
      pick(p);
    } catch (e) {
      fail(e, "create profile");
    } finally {
      setBusy(false);
    }
  };

  const remove = async (id: string) => {
    setBusy(true);
    try {
      const r = await invoke<{ dir?: string | null; keychain_hint?: boolean }>("delete_profile", { id });
      await reload();
      setSel("");
      setDraft(null);
      // Both of these are things we deliberately do NOT do silently.
      onReport(
        `Removed the profile.${r?.dir ? ` Its conversation history is still at ${r.dir} — delete it by hand if you want it gone.` : ""}` +
          (r?.keychain_hint
            ? " Its saved sign-in also stays in your login keychain (worktrees never touches credentials); remove it in Keychain Access if you want it gone."
            : ""),
      );
    } catch (e) {
      fail(e, "delete profile");
    } finally {
      setBusy(false);
      setArmed("");
    }
  };

  const setDefault = async (id: string | null) => {
    try {
      await invoke("set_default_profile", { id });
      await reload();
    } catch (e) {
      fail(e, "set default profile");
    }
  };

  const addSkillFromDisk = async () => {
    try {
      const dir = await openDialog({ directory: true, title: "Choose a skill directory" });
      if (!dir || typeof dir !== "string") return;
      const e = await invoke<SkillEntry>("skill_install_local", { path: dir });
      await reload();
      const caps = e?.capabilities ?? [];
      onReport(
        caps.length
          ? `Installed “${e.name}”. It asks for more than reading files: ${caps.join("; ")}`
          : `Installed “${e.name}”.`,
      );
    } catch (e) {
      fail(e, "install skill");
    }
  };

  const previewGit = async () => {
    setBusy(true);
    setPreview(null);
    try {
      const p = await invoke<GitPreview>("skill_preview_git", { url: gitUrl, rev: null });
      setPreview(p);
    } catch (e) {
      fail(e, "fetch skill");
    } finally {
      setBusy(false);
    }
  };

  const installGit = async (name: string) => {
    if (!preview) return;
    setBusy(true);
    try {
      await invoke("skill_install_git", { url: preview.url, rev: preview.rev || null, sha: preview.sha, name });
      setPreview(null);
      setGitUrl("");
      await reload();
      onReport(`Installed “${name}”, pinned to ${short(preview.sha)}.`);
    } catch (e) {
      fail(e, "install skill");
    } finally {
      setBusy(false);
    }
  };

  const removeSkill = async (name: string) => {
    try {
      const used = await invoke<string[] | null>("skill_remove", { name });
      await reload();
      onReport(
        used && used.length
          ? `Removed “${name}”. Still enabled in: ${used.join(", ")} — those profiles will warn on next launch.`
          : `Removed “${name}”.`,
      );
    } catch (e) {
      fail(e, "remove skill");
    } finally {
      setArmed("");
    }
  };

  const profiles = info?.profiles ?? [];
  const toggleSkill = (name: string) => {
    if (!draft) return;
    const have = draft.skills ?? [];
    setDraft({ ...draft, skills: have.includes(name) ? have.filter((s) => s !== name) : [...have, name] });
  };

  return (
    <>
      {info?.env_override ? (
        <section className="setting">
          <label>
            Overridden <span className="upd-tag warn">env</span>
          </label>
          <div className="hint">
            <code>WORKTREES_PROFILE={info.env_override}</code> is set in this app's environment, so it wins
            over everything below. Unset it for these choices to take effect.
          </div>
        </section>
      ) : null}

      <section className="setting">
        <label>Profiles</label>
        <div className="hint">
          A profile replaces your global <code>~/.claude</code> setup for sessions worktrees launches —
          rules, skills, MCP servers and model. Your normal terminal <code>claude</code> is untouched.
        </div>
        <div className="ver-rows">
          {profiles.length === 0 ? <div className="ver-row">No profiles yet.</div> : null}
          {profiles.map((p) => (
            <div key={p.id} className="ver-row">
              <button
                className={"settings-cat" + (sel === p.id ? " on" : "")}
                onClick={() => pick(p)}
                title={p.dir ?? undefined}
              >
                {p.name}
              </button>
              {info?.default_id === p.id ? <span className="upd-tag">default</span> : null}
              {info?.effective_id === p.id ? <span className="upd-tag">this repo</span> : null}
              {p.ever_launched === false ? (
                // The pane will say `Not logged in · Run /login` on first launch —
                // claude's credential is keyed to the config-dir path. Saying so
                // here is the difference between "expected" and "broken".
                <span className="upd-tag warn" title="First launch asks you to sign in once">
                  needs sign-in
                </span>
              ) : null}
            </div>
          ))}
        </div>
        <div className="ver-actions">
          <button className="ctrl sm" onClick={create} disabled={busy}>
            New profile
          </button>
          <button className="ctrl sm" onClick={() => setDefault(null)} disabled={busy || !info?.default_id}>
            Clear default
          </button>
        </div>
      </section>

      {draft ? (
        <>
          <section className="setting">
            <label>Name</label>
            <input
              type="text"
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              onBlur={() => save(draft)}
            />
            <label className="sub">Identifier</label>
            <div className="hint">
              <code>{draft.id}</code> — fixed for the life of the profile. Its directory path is also its
              saved-sign-in identity, so renaming the id would sign it out.
            </div>
          </section>

          <section className="setting">
            <label>Rules</label>
            <textarea
              className="profile-rules"
              rows={8}
              value={draft.rules ?? ""}
              placeholder="Be succinct. Prefer small diffs. …"
              onChange={(e) => setDraft({ ...draft, rules: e.target.value })}
              onBlur={() => save(draft)}
            />
            <div className="hint">
              Added to every session in this profile. Note it ADDS to your global{" "}
              <code>~/.claude/CLAUDE.md</code> (and anything that imports) — no profile can suppress those.
            </div>
          </section>

          <section className="setting">
            <label>Model</label>
            <input
              type="text"
              value={draft.model ?? ""}
              placeholder="(claude's default)"
              onChange={(e) => setDraft({ ...draft, model: e.target.value || null })}
              onBlur={() => save(draft)}
            />
          </section>

          <section className="setting">
            <label>Skills</label>
            <div className="hint">
              Enabled skills are symlinked into the profile, so editing one reaches a RUNNING session.
              (The “restart to apply” badge covers rules, model, MCP and settings — not skills.)
            </div>
            <div className="ver-rows">
              {skills.length === 0 ? <div className="ver-row">No skills installed.</div> : null}
              {skills.map((s) => (
                <div key={s.name} className="ver-row">
                  <label className="setting-check">
                    <input
                      type="checkbox"
                      checked={(draft.skills ?? []).includes(s.name)}
                      onChange={() => {
                        toggleSkill(s.name);
                      }}
                      onBlur={() => save(draft)}
                    />
                    {s.name}
                  </label>
                  {s.source?.kind === "git" ? (
                    <span className="ver-path" title={s.source.url}>
                      git @{short(s.source.sha)}
                    </span>
                  ) : null}
                  {s.capabilities?.length ? (
                    <span className="upd-tag warn" title={s.capabilities.join("; ")}>
                      {s.capabilities.length} capability
                    </span>
                  ) : null}
                  <button
                    className={"ctrl sm danger" + (armed === "skill:" + s.name ? " armed" : "")}
                    onClick={() => (armed === "skill:" + s.name ? removeSkill(s.name) : setArmed("skill:" + s.name))}
                  >
                    {armed === "skill:" + s.name ? "Really remove?" : "Remove"}
                  </button>
                </div>
              ))}
            </div>
            <label className="setting-check">
              <input
                type="checkbox"
                checked={!!draft.inherit_global_skills}
                onChange={(e) => save({ ...draft, inherit_global_skills: e.target.checked })}
              />
              Also load my global <code>~/.claude/skills</code>
            </label>
            <div className="ver-actions">
              <button className="ctrl sm" onClick={addSkillFromDisk} disabled={busy}>
                Add from folder…
              </button>
            </div>
            <div className="row2">
              <input
                type="text"
                value={gitUrl}
                placeholder="https://github.com/… (skill repository)"
                onChange={(e) => setGitUrl(e.target.value)}
              />
              <button className="ctrl sm" onClick={previewGit} disabled={busy || !gitUrl.trim()}>
                Review…
              </button>
            </div>
            {preview ? (
              <div className="ver-rows">
                <div className="ver-row">
                  Pinned to <code>{short(preview.sha, 12)}</code> — this exact commit is what installs.
                </div>
                {preview.skills.map((s) => (
                  <div key={s.name} className="ver-row">
                    <b>{s.name}</b>
                    <span className="ver-path">{s.description}</span>
                    {s.capabilities.length ? (
                      // A skill's description loads into every session before
                      // anything invokes it, and `allowed-tools` pre-authorises
                      // tool use. Read before installing, not after.
                      <span className="upd-tag warn" title={s.capabilities.join("; ")}>
                        asks for: {s.capabilities.join("; ")}
                      </span>
                    ) : null}
                    <button className="ctrl sm" onClick={() => installGit(s.name)} disabled={busy}>
                      Install
                    </button>
                  </div>
                ))}
                {preview.skills.map((s) => (
                  <pre key={"md:" + s.name} className="update-log">
                    {s.skill_md}
                  </pre>
                ))}
              </div>
            ) : null}
          </section>

          <section className="setting">
            <label>MCP servers</label>
            <label className="setting-check">
              <input
                type="checkbox"
                checked={!!draft.worktrees_mcp}
                onChange={(e) => save({ ...draft, worktrees_mcp: e.target.checked })}
              />
              Expose the worktrees MCP server (read-only tools for this repo)
            </label>
            <label className="setting-check">
              <input
                type="checkbox"
                checked={!!draft.inherit_global_mcp}
                onChange={(e) => save({ ...draft, inherit_global_mcp: e.target.checked })}
              />
              Also load my global MCP servers
            </label>
            <div className="hint">
              With this off, the profile's set is the whole set — that is what lets a profile REMOVE a
              global server rather than only add to it.
            </div>
          </section>

          <section className="setting">
            <label>This profile</label>
            <div className="ver-actions">
              <button className="ctrl sm" onClick={() => setDefault(draft.id)} disabled={info?.default_id === draft.id}>
                Make default
              </button>
              <button
                className="ctrl sm"
                onClick={() => draft.dir && revealItemInDir(draft.dir).catch((e) => fail(e, "reveal"))}
                disabled={!draft.dir || draft.ever_launched === false}
                title={draft.ever_launched === false ? "Nothing there until its first launch" : draft.dir ?? ""}
              >
                Reveal files
              </button>
              <button
                className={"ctrl sm danger" + (armed === "profile:" + draft.id ? " armed" : "")}
                onClick={() => (armed === "profile:" + draft.id ? remove(draft.id) : setArmed("profile:" + draft.id))}
              >
                {armed === "profile:" + draft.id ? "Really delete?" : "Delete"}
              </button>
            </div>
          </section>
        </>
      ) : null}

      {err ? <pre className="update-log">{err}</pre> : null}
    </>
  );
}

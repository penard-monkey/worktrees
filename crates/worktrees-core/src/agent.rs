//! Agents — the Claude session(s) running in a place, read from Claude Code's
//! own probe files.
//!
//! Claude Code writes ONE probe file per live session at
//! `<config dir>/sessions/<pid>.json` — `{ pid, cwd, status, name, tmux,
//! sessionId, parkedJobId, updatedAt, statusUpdatedAt, … }`. `status` ∈ { busy,
//! idle, waiting, shell, (missing) }; `cwd` is the session's working dir = the
//! worktree directory (pane-0 claude is launched in the place path), so it maps
//! 1:1 to a place's `path`. The file is rewritten on status TRANSITIONS only —
//! `updatedAt` can be minutes old while genuinely still busy — so nothing here
//! expires by age. Liveness is PID-alive (stale files for dead pids are never
//! cleaned on crash). Everything degrades to "no agent": missing/unreadable dir
//! → none; unparseable file → skipped; dead pid → skipped.
//!
//! This used to live in the app (the nav's busy/waiting dots). It moved here so
//! the MCP server's `place_status` and the app read ONE truth — an orchestrator
//! asking "is anyone working in that place?" must get the answer the dot shows.
//!
//! PARKED JOBS are the one place `status` lies. Parking a turn (handing it to a
//! background session) rewrites the interactive probe with `parkedJobId` set —
//! and if the turn was MID-FLIGHT, that rewrite leaves `status: "busy"` behind
//! and no further write ever clears it. Observed on claude 2.1.220: two sessions
//! sat green for 22h and 32h at 0% CPU. `busy_is_delegated` is the guard.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeProbe {
    pub pid: i32,
    pub cwd: String,
    pub status: String,
    /// The session's peer name (`--name`, `/rename`, or derived) — the address
    /// another session uses to message it. Absent on older claude versions.
    #[serde(default)]
    pub name: Option<String>,
    /// `<session>:@<window>.%<pane>` when the session runs inside tmux.
    #[serde(default)]
    pub tmux: Option<String>,
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
    /// `interactive`, `background`, …
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// Set when this session handed its turn to a background job. `Option` is
    /// what keeps the three park fields optional (serde maps a missing field on
    /// an Option to None); the type matters, because a probe written before the
    /// park feature carries none of them and a hard parse error would drop that
    /// session from the scan entirely — its dot with it.
    #[serde(default, rename = "parkedJobId")]
    pub parked_job_id: Option<String>,
    /// Time of the LAST write of any kind (a park write moves this alone).
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<i64>,
    /// Time of the last write that CHANGED `status`.
    #[serde(default, rename = "statusUpdatedAt")]
    pub status_updated_at: Option<i64>,
}

/// True when this probe's `busy` is residue from PARKING rather than live work.
///
/// The discriminator is `updated_at > status_updated_at`: the probe's last write
/// did not set the status it is carrying. A park write moves `updated_at` alone
/// (observed +15s and +891s past the busy transition on the two stuck sessions),
/// while a genuine busy transition writes both stamps in one go — every truly
/// working probe sampled had `updated_at == status_updated_at`. So a NEW turn in
/// a session that once parked a job still lights its dot, which is why this is
/// not simply "`parkedJobId` present → never busy": nothing proves Claude clears
/// that field, and a place whose dot goes permanently dark is the worse failure.
///
/// No cross-probe join with the background session, deliberately. While a parked
/// job is genuinely running its OWN probe carries the same `cwd` (it is a fork of
/// this session), so the dot stays lit through that probe — and this stays a pure
/// per-probe predicate that also covers the case where the bg probe is gone.
/// Cost: a parked job running with no local session at all (a purely remote
/// bridge) shows no dot, which is the honest answer for a pane sitting idle.
pub fn busy_is_delegated(p: &ClaudeProbe) -> bool {
    p.parked_job_id.is_some()
        && matches!((p.updated_at, p.status_updated_at), (Some(u), Some(s)) if u > s)
}

/// True when `pid` is a live process. `kill(pid, 0)` sends no signal but runs the
/// permission/existence checks: 0 → alive; -1 with errno ESRCH → dead. EPERM
/// (exists but not ours) still means alive. Never mutates the target.
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: kill with signal 0 performs no action beyond error checking.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    // rc == -1: alive iff the failure is EPERM (exists, not permitted), not ESRCH.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The state a reader should act on: the probe's `status`, except that a
/// parked-away `busy` reads as `delegated` (see `busy_is_delegated`).
pub fn effective_state(p: &ClaudeProbe) -> String {
    if p.status == "busy" && busy_is_delegated(p) {
        "delegated".to_string()
    } else {
        p.status.clone()
    }
}

/// Scan `<config dir>/sessions/*.json` for EVERY config root and keep only
/// LIVE-pid probes with a cwd. Any I/O or parse failure degrades to "skipped".
///
/// Every root, not just `~/.claude`: a profiled session writes its probes under
/// the profile's own dir, so a $HOME-only scan would leave profiled places
/// permanently agent-less — a silent degradation with nothing to explain it.
/// Union is safe because each probe carries its own `cwd` and `pid`, so it is
/// self-describing: nobody needs to know which profile a place is bound to.
pub fn live_probes() -> Vec<ClaudeProbe> {
    let mut out = Vec::new();
    for root in crate::profile::claude_config_dirs_all() {
        let entries = match std::fs::read_dir(root.join("sessions")) {
            Ok(e) => e,
            Err(_) => continue, // dir missing/unreadable → nothing from this root
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let probe: ClaudeProbe = match serde_json::from_slice(&bytes) {
                Ok(p) => p, // missing pid/cwd/status → parse fails → skip
                Err(_) => continue,
            };
            if probe.cwd.is_empty() || !pid_alive(probe.pid) {
                continue; // dead pid (or crashed-session stale file) → skip
            }
            out.push(probe);
        }
    }
    out
}

/// One agent as reported to a caller (the MCP `place_status` tool). A projection
/// of the probe: what an orchestrator needs to address the session (`name`), to
/// decide whether to brief it (`state`), and to find its pane (`tmux`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Agent {
    /// `busy` · `waiting` (on the user) · `idle` · `delegated` (parked-away
    /// busy) · `shell` · anything else claude writes, verbatim.
    pub state: String,
    pub pid: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmux: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Rank for "most active first": the first agent in the list is the one whose
/// state answers "is anyone working here?".
fn rank(state: &str) -> u8 {
    match state {
        "busy" => 0,
        "waiting" => 1,
        "delegated" => 2,
        "idle" => 3,
        _ => 4,
    }
}

/// The agents running in the place at `path`, most active first. Pure: takes
/// the probes so the ordering is testable without a filesystem.
///
/// Matched on `cwd == path` — the same key the app's dots use. Paths are
/// compared raw: a probe's cwd and a place's `path` both derive from the same
/// realpath-normalized worktree dir. A subagent fork or a `claude` started by
/// hand in the dock's shell shares the cwd and is listed too; `tmux` and `kind`
/// tell them apart.
pub fn agents_at(probes: &[ClaudeProbe], path: &str) -> Vec<Agent> {
    let mut out: Vec<Agent> = probes
        .iter()
        .filter(|p| p.cwd == path)
        .map(|p| Agent {
            state: effective_state(p),
            pid: p.pid,
            name: p.name.clone(),
            tmux: p.tmux.clone(),
            session_id: p.session_id.clone(),
            kind: p.kind.clone(),
            version: p.version.clone(),
        })
        .collect();
    out.sort_by_key(|a| (rank(&a.state), a.pid));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(s: &str) -> ClaudeProbe {
        serde_json::from_str(s).expect("probe must parse")
    }

    /// The stuck-green bug, verbatim: the two probes that sat `busy` for 22h and
    /// 32h with their sessions at an idle prompt, next to the two genuinely-busy
    /// probes sampled at the same moment. The park write moves `updatedAt` alone;
    /// a real busy transition writes both stamps together.
    #[test]
    fn a_parked_away_busy_probe_does_not_light_the_dot() {
        // nautiseven: parked mid-turn, +15s between the busy write and the park.
        let stuck = probe(
            r#"{"pid":16304,"cwd":"/w/nautiseven","status":"busy",
                "parkedJobId":"d8d56f6f","updatedAt":1786804861210,
                "statusUpdatedAt":1786804845800}"#,
        );
        assert!(busy_is_delegated(&stuck));
        assert_eq!(effective_state(&stuck), "delegated");

        // geant4: same shape, +891s.
        let stuck2 = probe(
            r#"{"pid":28024,"cwd":"/w/geant4","status":"busy","parkedJobId":"449d44a7",
                "updatedAt":1786767105635,"statusUpdatedAt":1786766214904}"#,
        );
        assert!(busy_is_delegated(&stuck2));

        // Genuinely working, never parked — the dot must stay lit.
        let working = probe(
            r#"{"pid":36804,"cwd":"/w/sync-macs","status":"busy","parkedJobId":null,
                "updatedAt":1786884619732,"statusUpdatedAt":1786884619732}"#,
        );
        assert!(!busy_is_delegated(&working));
        assert_eq!(effective_state(&working), "busy");

        // A NEW turn in a session that once parked a job: `parkedJobId` may well
        // linger, so only the stamps can tell this from the stuck case above.
        let working_after_park = probe(
            r#"{"pid":5387,"cwd":"/w/white-label","status":"busy","parkedJobId":"2b899e3a",
                "updatedAt":1786884619732,"statusUpdatedAt":1786884619732}"#,
        );
        assert!(!busy_is_delegated(&working_after_park));
    }

    /// The three park fields are additions to a file Claude Code owns. A probe
    /// written without them must still parse — a hard error here would drop that
    /// session from the scan and take its dot with it.
    #[test]
    fn a_probe_without_the_park_fields_still_parses_and_stays_busy() {
        let old = probe(r#"{"pid":1,"cwd":"/w/x","status":"busy"}"#);
        assert_eq!(old.parked_job_id, None);
        assert!(!busy_is_delegated(&old), "unknown stamps must never suppress");

        // Half-written is the same story: no pair of stamps, no judgement.
        let partial = probe(
            r#"{"pid":1,"cwd":"/w/x","status":"busy","parkedJobId":"abc","updatedAt":9}"#,
        );
        assert!(!busy_is_delegated(&partial));
    }

    /// A real probe (claude 2.1.259) carries the addressing fields — and older
    /// ones don't, so every one of them is optional.
    #[test]
    fn a_real_probe_yields_name_and_tmux() {
        let p = probe(
            r#"{"pid":34446,"sessionId":"a8f9fa32","cwd":"/w/valleos/.worktrees/communications",
                "version":"2.1.259","kind":"interactive","tmux":"valleos-communications:@14.%16",
                "name":"communications-61","nameSource":"derived","status":"idle",
                "updatedAt":1788391209429,"statusUpdatedAt":1788391209429}"#,
        );
        let agents = agents_at(&[p], "/w/valleos/.worktrees/communications");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name.as_deref(), Some("communications-61"));
        assert_eq!(agents[0].tmux.as_deref(), Some("valleos-communications:@14.%16"));
        assert_eq!(agents[0].state, "idle");
        // A different place sees nothing.
        assert!(agents_at(&agents_at_probes(), "/w/elsewhere").is_empty());
    }

    fn agents_at_probes() -> Vec<ClaudeProbe> {
        vec![probe(r#"{"pid":1,"cwd":"/w/a","status":"idle"}"#)]
    }

    /// Most active first: the first entry is the one whose state answers "is
    /// anyone working here?" — an idle fork must not hide a busy session.
    #[test]
    fn agents_are_ordered_most_active_first() {
        let probes = vec![
            probe(r#"{"pid":3,"cwd":"/w/a","status":"idle"}"#),
            probe(r#"{"pid":2,"cwd":"/w/a","status":"busy","parkedJobId":"x","updatedAt":5,"statusUpdatedAt":1}"#),
            probe(r#"{"pid":1,"cwd":"/w/a","status":"waiting"}"#),
            probe(r#"{"pid":9,"cwd":"/w/b","status":"busy"}"#),
        ];
        let states: Vec<String> = agents_at(&probes, "/w/a").into_iter().map(|a| a.state).collect();
        assert_eq!(states, vec!["waiting", "delegated", "idle"]);
        // `agent` JSON leaves absent fields out rather than writing nulls.
        let j = serde_json::to_value(&agents_at(&probes, "/w/b")[0]).unwrap();
        assert_eq!(j, serde_json::json!({"state":"busy","pid":9}));
    }
}

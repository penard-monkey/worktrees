//! A picture in a shared session (`shared_image`): what lets a guest see the
//! images the owner's chat shows in the agent's replies.
//!
//! The owner's chat asks `read_place_image` with a repo and a slug, which a
//! guest never names and holds no `read` for. Here the SHARE names the place,
//! and the rule a path meets is the same one: `safepath::place_image` under
//! that place's root, opened `O_NOFOLLOW` and held to the identity checked,
//! then `check_image` (`files::place_image_at` is the body both share).
//!
//! **What is shared is a session, not a place**, so a path must also be one
//! the session SHOWS: turn `turn` of the bound transcript has to draw it,
//! either as `![alt](path)` in the agent's own text or as a Read of it that
//! did not fail. Without that, a guest could walk the place by guessing
//! names: another session's screenshots, the owner's uploads, whether a file
//! exists at all. The turn is the client's to name (`shared_turns` gave it the
//! `seq`), so a check reads that turn's bytes and not the transcript.
//!
//! **A person's message counts only for a picture the HOST stored.** A
//! guest's own words reach the transcript as a user message when the owner
//! presses *Use these* (the frame names the guests, and the owner can paste
//! anything), so a path in a person's text proves nothing by itself: counting
//! it would let a guest write the path they want read. What a guest's words
//! cannot reach is the host's ledger (`attach::LEDGER`, in serve's config dir,
//! written by `agent_attach` for every IMAGE it stores, never by an agent and
//! never in the place). So a person's message (its prompt, or one sent while
//! the agent worked) shows a path, as its `[attached: …]` tail or as
//! `![alt](path)`, only when the ledger says the host stored that picture in
//! this place DURING the binding (from `bound_at`, and by the end of an ended
//! one), and the file is unchanged since (its own times no later than the
//! record). *Use these*' upload is a text file, never recorded; a picture a
//! guest names that the owner did not upload in this share is refused as
//! before. A picture pasted into Claude's TERMINAL is an image block in the
//! transcript that the owner's chat does not draw either, so it is not here.
//!
//! **An ended binding shows what it showed** (§3.3): an image changed on disk
//! after the binding ended is another session's work, and is refused.
//!
//! **A Read of a file OUTSIDE the place** is refused here as `outside this
//! place`, like any other path: the host never reads that path for a guest.
//! Its picture is the one Claude was SHOWN, in the transcript, which the
//! owner's chat draws from `agent_call_image` (console-only, because that row
//! names a place, not a session). A guest's is `shared_call_image`
//! ([`rows::call_image`]): the call must be one the shared session holds below
//! its limit (the history index's, so another session's id and a turn past an
//! ended binding are not), a Claude Read that did not fail, and its block
//! passes `check_image` and the size cap. Nothing on disk is opened.

use std::path::Path;

use worktrees_core::attach;
use worktrees_core::authz::{Access, Requirement};
use worktrees_core::pathclass::{self, Classifier};
use worktrees_core::turns::{Step, Turn};

/// The path is not one the named turn draws. One answer for a turn that is
/// not there, a path it does not show and a file that does not exist, given
/// before the disk is looked at.
pub const ERR_NOT_SHOWN_HERE: &str = "not shown in this shared session";

/// No picture Claude was shown for that call in this shared session: one
/// answer for an id it does not hold, a call that is no Read, a failed Read
/// and a Read whose result holds no image.
pub const ERR_NO_CALL_PICTURE: &str = "not shown: no picture of this call in this shared session";

/// The picture on disk is newer than the shared session.
pub const ERR_CHANGED: &str = "this picture was changed after the shared session ended";

/// Characters of an alt text, a reference's name or a destination: past it
/// the text is not read as an image, so a long run of `![` costs no more than
/// this each.
const PART_MAX: usize = 4096;

/// The characters a backslash takes the meaning from in markdown.
fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation()
}

/// The end of a `[…]` whose `[` is just before `s`: its text and what follows
/// the `]`. Nested brackets and escapes are read, as markdown reads them.
fn bracketed(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut skip = false;
    for (n, (i, c)) in s.char_indices().enumerate() {
        if n > PART_MAX {
            return None;
        }
        if std::mem::take(&mut skip) {
            continue;
        }
        match c {
            '\\' => skip = true,
            '[' => depth += 1,
            ']' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            ']' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The link destination `s` starts with, as written: `<…>`, or up to white
/// space or the `)` that closes the link, with its escapes taken out.
fn destination(s: &str) -> Option<String> {
    let s = s.trim_start();
    let (angle, s) = match s.strip_prefix('<') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let mut out = String::new();
    let mut depth = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek().copied().is_some_and(is_punct) => out.push(chars.next()?),
            '\n' if angle => return None,
            '>' if angle => break,
            c if angle => out.push(c),
            c if c.is_whitespace() || c.is_control() => break,
            '(' => {
                depth += 1;
                out.push(c);
            }
            ')' if depth == 0 => break,
            ')' => {
                depth -= 1;
                out.push(c);
            }
            c => out.push(c),
        }
        if out.len() > PART_MAX {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// `[name]: destination` lines, for `![alt][name]`. The first of a name wins.
fn definitions(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let t = l.trim_start_matches(' ');
            if l.len() - t.len() > 3 {
                return None;
            }
            let (name, rest) = bracketed(t.strip_prefix('[')?)?;
            Some((label(name), destination(rest.strip_prefix(':')?)?))
        })
        .collect()
}

fn label(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// `%xx` decoded, as the client's `decodeURIComponent` does: whole, or the
/// text as it was when an escape is not one.
fn percent_decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let hex = |n: usize| b.get(n).and_then(|c| (*c as char).to_digit(16));
        let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2)) else { return s.to_string() };
        out.push((h * 16 + l) as u8);
        i += 3;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// The image destinations a markdown text draws, decoded as the client
/// decodes them: `![alt](dest "title")`, `![alt](<dest>)`, `![alt][name]` and
/// `![name]`. Code is not told apart: an image written in a code span is the
/// agent's own text too, and its path is judged like any other.
pub fn image_paths(text: &str) -> Vec<String> {
    let mut defs: Option<Vec<(String, String)>> = None;
    let mut out = Vec::new();
    for (at, _) in text.match_indices("![") {
        let Some((alt, rest)) = bracketed(&text[at + 2..]) else { continue };
        let dest = if let Some(r) = rest.strip_prefix('(') {
            destination(r)
        } else {
            let name = match rest.strip_prefix('[').and_then(bracketed) {
                Some((n, _)) if !n.trim().is_empty() => n,
                _ => alt,
            };
            let want = label(name);
            defs.get_or_insert_with(|| definitions(text)).iter().find(|(n, _)| *n == want).map(|(_, d)| d.clone())
        };
        out.extend(dest.map(|d| percent_decoded(d.trim())));
    }
    out
}

/// `t` draws the picture at `path`: in the agent's text, or as a Read of it.
fn draws(t: &Turn, path: &str) -> bool {
    t.steps.iter().any(|s| match s {
        Step::Text { text, .. } => image_paths(text).iter().any(|p| p == path),
        Step::Tool { name, summary: Some(s), failed: false, .. } => name == "Read" && s == path,
        _ => false,
    })
}

/// The host-shaped paths a sent line's ` [attached: …]` tail names, as the
/// client's `splitAttached` reads them; none when the tail is not that shape.
fn attached(line: &str) -> Vec<&str> {
    let ok = |p: &str| {
        let Some(name) = p.strip_prefix(".worktrees-attachments/") else { return false };
        let b = name.as_bytes();
        let ext = name.get(26..).unwrap_or("");
        b.len() > 26
            && b[..8].iter().all(u8::is_ascii_digit)
            && b[8] == b'-'
            && b[9..25].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
            && b[25] == b'.'
            && (2..=4).contains(&ext.len())
            && ext.bytes().all(|c| c.is_ascii_lowercase())
    };
    let Some(tail) = line.strip_suffix(']') else { return Vec::new() };
    let Some((_, list)) = tail.rsplit_once(" [attached: ") else { return Vec::new() };
    let paths: Vec<&str> = list.split(", ").collect();
    if paths.iter().all(|p| ok(p)) { paths } else { Vec::new() }
}

/// A PERSON's message in `t` names `path`: its `[attached: …]` tail or an
/// image its markdown draws. Not enough alone: see the module's header.
fn person_names(t: &Turn, path: &str) -> bool {
    let steps = t.steps.iter().filter_map(|s| match s {
        Step::User { text, .. } => Some(text.as_str()),
        _ => None,
    });
    t.user.as_deref().into_iter().chain(steps).any(|x| attached(x).contains(&path) || image_paths(x).iter().any(|p| p == path))
}

/// What a guest may be shown of the file an image path resolved to: one a
/// plain `read` reaches (`pathclass::requirement`), so never a secret, git's
/// own or worktrees' own. A guest holds no `read`, so this stands in for
/// `Ctx::may`. A path under no root cannot be classed, and is refused.
fn class_allows(classes: &Classifier, canon: &Path) -> bool {
    classes.class_for(canon, Access::Read).is_some_and(|c| pathclass::requirement(c, Access::Read) == Requirement::Row)
}

/// The newest change (ms) a file's own times tell: its contents', or its
/// name's and mode's, which a copy that keeps the old mtime still moves.
fn changed_ms(canon: &Path) -> Result<i64, String> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(canon).map_err(|_| "not found".to_string())?;
    Ok((m.mtime() * 1000 + m.mtime_nsec() / 1_000_000).max(m.ctime() * 1000 + m.ctime_nsec() / 1_000_000))
}

/// The body of the row (`dispatch.rs`), after the resolver's `share(share)`
/// check: the share names the caller. The guest must also be ALLOWED on it.
pub mod rows {
    use super::*;
    use crate::serve::dispatch::Call;
    use crate::serve::history;
    use crate::serve::shares::{place_dir, place_is_there, rows::all_roots, BindingState};
    use worktrees_api::files;

    pub fn image(cx: &Call, share: String, binding: Option<usize>, turn: usize, path: String, etag: Option<String>) -> Result<files::PlaceImage, String> {
        let s = history::rows::share(cx, &share)?;
        let b = history::binding_of(&s, binding)?;
        let not_shown = || format!("{ERR_NOT_SHOWN_HERE}: {path}");
        let turns = history::turns_at(&cx.ctx.config_dir, b, turn)?;
        // The place the owner shared, still: a project unregistered or a place
        // remade under the same name waits for the poll to drop the share.
        let roots = all_roots(cx);
        if !roots.contains(&s.project) || !place_is_there(&s) {
            return Err(not_shown());
        }
        let place = place_dir(&s.project, &s.slug);
        // An ended binding with no end time shows nothing it cannot date.
        let until_ms = (b.state == BindingState::Ended).then(|| b.ended_at.unwrap_or(0) * 1000 + 999);
        // The agent's own picture, or a person's that the host stored during
        // the binding: then `uploaded` is when, and the file must be no newer.
        let uploaded = if turns.iter().any(|t| draws(t, &path)) {
            None
        } else if turns.iter().any(|t| person_names(t, &path)) {
            let at = attach::uploaded(&cx.ctx.config_dir.join(attach::LEDGER), &place, &path);
            let within = |ms: &i64| *ms >= b.bound_at * 1000 && until_ms.is_none_or(|t| *ms <= t);
            Some(at.filter(within).ok_or_else(not_shown)?)
        } else {
            return Err(not_shown());
        };
        let canon: Vec<std::path::PathBuf> = roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect();
        let classes = Classifier::new(&canon);
        let changed = || format!("{ERR_CHANGED}: {path}");
        let allow = |f: &Path| {
            if !class_allows(&classes, f) {
                return Err(not_shown());
            }
            if uploaded.is_some_and(|ms| changed_ms(f).map_or(true, |c| c > ms)) {
                return Err(not_shown());
            }
            match until_ms {
                Some(t) if changed_ms(f)? > t => Err(changed()),
                _ => Ok(()),
            }
        };
        let img = files::place_image_at(&place, s.slug == "(main)", &path, files::READ_BASE64_MAX, etag.as_deref(), &allow)?;
        // Again for the handle that was read: a file written between the two.
        if uploaded.is_some_and(|ms| img.mtime as i64 > ms) {
            return Err(not_shown());
        }
        if until_ms.is_some_and(|t| img.mtime as i64 > t) {
            return Err(changed());
        }
        Ok(img)
    }

    /// `shared_call_image`: the picture a Claude Read of call `call_id` was
    /// shown, from the shared session's own transcript (the module's header).
    pub fn call_image(cx: &Call, share: String, binding: Option<usize>, call_id: String) -> Result<serde_json::Value, String> {
        use worktrees_core::turns;
        let s = history::rows::share(cx, &share)?;
        let b = history::binding_of(&s, binding)?;
        let gone = || ERR_NO_CALL_PICTURE.to_string();
        if b.harness != "claude" || call_id.is_empty() {
            return Err(gone());
        }
        let (turn, lines) = history::call_lines(&cx.ctx.config_dir, b, &call_id).map_err(|_| gone())?;
        let read_ok = turns::history_turns(&b.harness, &lines, turns::TURN_TEXT_MAX).iter().any(|(t, _)| {
            t.steps.iter().any(|st| matches!(st, Step::Tool { name, id, failed: false, .. } if name == "Read" && *id == call_id))
        });
        if !read_ok {
            return Err(gone());
        }
        let img = turns::claude_call_image(&lines, &call_id).ok_or_else(gone)?;
        if !img.truncated {
            let bytes = attach::decode(&img.b64).map_err(|_| "not shown: the picture Claude was shown is not readable".to_string())?;
            worktrees_core::safepath::check_image(&bytes, &img.mime, "the picture Claude was shown")?;
        }
        Ok(serde_json::json!({ "call_id": call_id, "turn": turn, "mime": img.mime, "b64": img.b64, "size": img.size, "truncated": img.truncated }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::dispatch::{CONSOLE_GUEST_ROWS, GUEST_READS, GUEST_ROWS, TABLE};
    use crate::serve::resolve as res;
    use crate::serve::shares::serve_tests::Host;
    use serde_json::{json, Value};
    use std::fs;
    use std::path::PathBuf;
    use worktrees_core::authz::{Caller, Capability};

    /// A 1×1 PNG: what `check_image` reads of it is its header.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

    fn user(text: &str) -> String {
        json!({"type": "user", "uuid": text, "timestamp": "2026-11-24T10:00:00Z", "origin": {"kind": "human"},
            "message": {"role": "user", "content": text}}).to_string() + "\n"
    }

    fn reply(text: &str) -> String {
        json!({"type": "assistant", "uuid": "a", "timestamp": "2026-11-24T10:00:01Z",
            "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}).to_string() + "\n"
    }

    fn read(id: &str, path: &str) -> String {
        json!({"type": "assistant", "uuid": "a", "timestamp": "2026-11-24T10:00:01Z",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": id, "name": "Read", "input": {"file_path": path}}]}}).to_string() + "\n"
    }

    fn result(id: &str, failed: bool) -> String {
        json!({"type": "user", "uuid": "r", "timestamp": "2026-11-24T10:00:02Z",
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": "x", "is_error": failed}]}}).to_string() + "\n"
    }

    fn png(p: PathBuf) -> PathBuf {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, PNG).unwrap();
        p
    }

    /// A share of place `slug` of Host's project `a` with Bret let in, bound
    /// to a Claude whose transcript is `text`.
    fn shared(h: &Host, slug: &str, session: &str, text: &str) -> String {
        fs::write(h.top.join(format!("{session}.jsonl")), text).unwrap();
        h.live(slug, session, 0);
        let bret = Host::principal(&h.bret);
        let id = h.console("share_create", json!({"repo": h.a, "slug": slug, "guests": [bret]})).unwrap()["id"].as_str().unwrap().to_string();
        h.console("share_allow", json!({"share": id, "principal": bret})).unwrap();
        id
    }

    fn ask(h: &Host, who: &Caller, share: &str, turn: usize, path: &str) -> Result<Value, Value> {
        h.run(who, "shared_image", json!({"share": share, "turn": turn, "path": path}))
    }

    fn said(r: Result<Value, Value>) -> String {
        r.expect_err("refused").as_str().unwrap().to_string()
    }

    fn feat(h: &Host) -> PathBuf {
        Path::new(&h.a).join(".worktrees/feat")
    }

    /// It is a guest row, a quiet read, and not one the console makes.
    #[test]
    fn the_row_is_a_guest_read() {
        let row = TABLE.iter().find(|r| r.name == "shared_image").unwrap();
        assert_eq!(row.cap, Some(Capability::Guest));
        assert!(GUEST_ROWS.contains(&"shared_image") && GUEST_READS.contains(&"shared_image") && !CONSOLE_GUEST_ROWS.contains(&"shared_image"));
        let o: Vec<&str> = row.objects.iter().flat_map(|o| o.args()).collect();
        assert_eq!(o, vec!["share"], "keyed by the share: no repo, no slug");
    }

    /// The owner's report: the pictures in the agent's reply, relative,
    /// absolute and percent-encoded, and a Read inside the place. The etag
    /// answers `unchanged` with no bytes, and a changed file is sent again.
    #[test]
    fn a_guest_sees_the_images_the_shared_session_shows() {
        let h = Host::new("img-shows");
        let home = png(feat(&h).join(".worktrees-attachments/agent/disk-home.png"));
        let abs = png(feat(&h).join("shots/abs.png"));
        png(feat(&h).join("shots/my pic.png"));
        let r = png(feat(&h).join("shots/read.png"));
        let text = user("draw it")
            + &reply(&format!("**Home**: every project\n![Home disk view](.worktrees-attachments/agent/disk-home.png)\n\n![Abs]({})\n![Enc](shots/my%20pic.png)", abs.display()))
            + &read("c1", r.to_str().unwrap())
            + &result("c1", false);
        let id = shared(&h, "feat", "s-1", &text);
        let v = ask(&h, &h.bret, &id, 0, ".worktrees-attachments/agent/disk-home.png").unwrap();
        assert_eq!((v["mime"].as_str(), v["b64"].as_str(), v["unchanged"].as_bool()), (Some("image/png"), Some(worktrees_api::files::b64_encode(PNG).as_str()), Some(false)));
        for p in [abs.to_str().unwrap(), "shots/my pic.png", r.to_str().unwrap()] {
            assert_eq!(ask(&h, &h.bret, &id, 0, p).unwrap()["mime"], json!("image/png"), "{p}");
        }
        let again = |etag: &Value| h.run(&h.bret, "shared_image", json!({"share": id, "turn": 0, "path": ".worktrees-attachments/agent/disk-home.png", "etag": etag})).unwrap();
        let held = again(&v["etag"]);
        assert_eq!((held["unchanged"].as_bool(), held["b64"].as_str()), (Some(true), Some("")));
        fs::write(&home, [PNG, b"more"].concat()).unwrap();
        assert_eq!(again(&v["etag"])["unchanged"], json!(false), "a changed file is sent again");
    }

    /// The share is of a session: a picture in the same place that this
    /// session does not show is refused, whether or not it exists, and so is
    /// one only another turn, a person's message or a failed Read names.
    #[test]
    fn a_path_the_session_does_not_show_is_refused_in_one_voice() {
        let h = Host::new("img-notshown");
        for p in ["private.png", "later.png", "mine.png", "failed.png", "linked.png"] {
            png(feat(&h).join(p));
        }
        let failed = feat(&h).join("failed.png");
        let text = user("here is ![mine](mine.png)")
            + &reply("Done. See private.png, and [a link](linked.png).")
            + &read("c1", failed.to_str().unwrap())
            + &result("c1", true)
            + &user("next")
            + &reply("![later](later.png)");
        let id = shared(&h, "feat", "s-1", &text);
        let words = |p: &str| format!("{ERR_NOT_SHOWN_HERE}: {p}");
        for p in ["private.png", "nothing-there.png", "later.png", "mine.png", "linked.png", failed.to_str().unwrap()] {
            assert_eq!(said(ask(&h, &h.bret, &id, 0, p)), words(p));
        }
        assert!(ask(&h, &h.bret, &id, 1, "later.png").is_ok(), "the turn that draws it");
        assert_eq!(said(ask(&h, &h.bret, &id, 99, "later.png")), words("later.png"), "no such turn");
    }

    /// Every path the place rule refuses, though the agent DID write it: out
    /// of the place, `..`, a link to a file, a link to a folder, another place,
    /// a file that is no image, a URL, a secret, git's own.
    #[test]
    fn the_places_own_rule_holds_for_a_path_the_session_shows() {
        let h = Host::new("img-rule");
        let out = png(h.top.join("out/secret.png"));
        let sib = png(Path::new(&h.a).join(".worktrees/other/sib.png"));
        let main = png(Path::new(&h.a).join("main.png"));
        let real = png(feat(&h).join("real.png"));
        std::os::unix::fs::symlink(&out, feat(&h).join("link.png")).unwrap();
        std::os::unix::fs::symlink(&real, feat(&h).join("inlink.png")).unwrap();
        std::os::unix::fs::symlink(h.top.join("out"), feat(&h).join("dir")).unwrap();
        fs::write(feat(&h).join("notes.txt"), "x").unwrap();
        png(feat(&h).join(".git/x.png"));
        png(feat(&h).join(".env.png"));
        let (out, sib, main) = (out.to_str().unwrap(), sib.to_str().unwrap(), main.to_str().unwrap());
        let cases = [
            (out, "outside this place"),
            (sib, "outside this place"),
            (main, "outside this place"),
            ("../other/sib.png", "outside this place"),
            ("a/../../other/sib.png", "outside this place"),
            ("link.png", "outside this place"),
            ("inlink.png", "outside this place"),
            ("dir/secret.png", "outside this place"),
            ("dir/nope.png", "outside this place"),
            ("gone.png", "not found"),
            ("notes.txt", "not an image"),
            ("https://example.com/a.png", "not a file in this place"),
            (".git/x.png", ERR_NOT_SHOWN_HERE),
            (".env.png", ERR_NOT_SHOWN_HERE),
        ];
        let md: String = cases.iter().map(|(p, _)| format!("![x]({p})\n")).collect();
        // A Read of a file outside the place is the console's picture alone.
        let text = user("go") + &reply(&md) + &read("c1", out) + &result("c1", false);
        let id = shared(&h, "feat", "s-1", &text);
        for (p, want) in cases {
            let got = said(ask(&h, &h.bret, &id, 0, p));
            assert_eq!(got, format!("{want}: {p}"));
        }
        // A picture too big to decode, refused by its header.
        let mut big = PNG.to_vec();
        big[16..24].copy_from_slice(&[0, 0, 0x27, 0x10, 0, 0, 0x27, 0x10]);
        fs::write(feat(&h).join("gone.png"), big).unwrap();
        assert!(said(ask(&h, &h.bret, &id, 0, "gone.png")).contains("not shown"));
    }

    /// `(main)`'s root holds every other place's tree: nothing under
    /// `.worktrees/` is `(main)`'s, however it is spelled.
    #[test]
    fn mains_worktrees_folder_is_not_its_own() {
        let h = Host::new("img-main");
        png(Path::new(&h.a).join("own.png"));
        let sib = png(Path::new(&h.a).join(".worktrees/other/sib.png"));
        let sib = sib.to_str().unwrap();
        let text = user("go") + &reply(&format!("![o](own.png) ![s](.worktrees/other/sib.png) ![d](./.worktrees/other/sib.png) ![a]({sib})"));
        let id = shared(&h, "(main)", "s-1", &text);
        assert!(ask(&h, &h.bret, &id, 0, "own.png").is_ok());
        for p in [".worktrees/other/sib.png", "./.worktrees/other/sib.png", sib] {
            assert_eq!(said(ask(&h, &h.bret, &id, 0, p)), format!("outside this place: {p}"));
        }
    }

    /// Who: not another guest's share, not before being let in, not after
    /// being removed or the share stopped, and not the console.
    #[test]
    fn only_a_guest_let_in_on_the_share_is_answered() {
        let h = Host::new("img-who");
        png(feat(&h).join("a.png"));
        let text = user("go") + &reply("![a](a.png)");
        let id = shared(&h, "feat", "s-1", &text);
        assert!(ask(&h, &h.bret, &id, 0, "a.png").is_ok());
        assert_eq!(ask(&h, &h.eve, &id, 0, "a.png"), Err(json!(res::ERR_NOT_FOUND)), "not named on it");
        assert_eq!(ask(&h, &h.bret, "s_nope", 0, "a.png"), Err(json!(res::ERR_NOT_FOUND)));
        assert!(ask(&h, &Caller::console(), &id, 0, "a.png").is_err(), "the console reads the place's own");
        // Named, not let in.
        let bret = Host::principal(&h.bret);
        fs::write(h.top.join("s-2.jsonl"), &text).unwrap();
        h.live("other", "s-2", 0);
        png(Path::new(&h.a).join(".worktrees/other/a.png"));
        let two = h.console("share_create", json!({"repo": h.a, "slug": "other", "guests": [bret]})).unwrap()["id"].as_str().unwrap().to_string();
        assert!(said(ask(&h, &h.bret, &two, 0, "a.png")).contains("not let in yet"));
        h.run(&h.bret, "share_join", json!({"share": two})).unwrap();
        assert!(said(ask(&h, &h.bret, &two, 0, "a.png")).contains("not let in yet"), "asking is not being let in");
        // Stopped, then removed.
        h.console("share_stop", json!({"share": two})).unwrap();
        assert_eq!(ask(&h, &h.bret, &two, 0, "a.png"), Err(json!(res::ERR_NOT_FOUND)));
        h.console("share_remove_guest", json!({"share": id, "principal": bret})).unwrap();
        assert_eq!(ask(&h, &h.bret, &id, 0, "a.png"), Err(json!(res::ERR_NOT_FOUND)));
    }

    /// A place removed and made again under the same name is not the one the
    /// owner shared, even before the poll drops the share; nor is one whose
    /// project is no longer registered.
    #[test]
    fn a_remade_place_or_an_unregistered_project_shows_nothing() {
        let mut h = Host::new("img-remade");
        png(feat(&h).join("a.png"));
        let id = shared(&h, "feat", "s-1", &(user("go") + &reply("![a](a.png)")));
        assert!(ask(&h, &h.bret, &id, 0, "a.png").is_ok());
        // Unregistered, though a project around it still is: that one would
        // class the file as its own.
        let around = vec![fs::canonicalize(&h.top).unwrap().to_string_lossy().into_owned()];
        let roots = std::mem::replace(&mut h.world.roots, around);
        assert_eq!(said(ask(&h, &h.bret, &id, 0, "a.png")), format!("{ERR_NOT_SHOWN_HERE}: a.png"), "unregistered");
        h.world.roots = roots;
        assert!(ask(&h, &h.bret, &id, 0, "a.png").is_ok());
        crate::serve::shares::remake(&feat(&h));
        png(feat(&h).join("a.png"));
        assert_eq!(said(ask(&h, &h.bret, &id, 0, "a.png")), format!("{ERR_NOT_SHOWN_HERE}: a.png"));
    }

    /// §3.3: an ended session shows what it showed. A turn past its offset
    /// draws nothing, the private session after `/clear` gives no picture, and
    /// one a later session wrote over is refused, its mtime put back or not.
    #[test]
    fn an_ended_session_shows_only_what_it_showed() {
        let h = Host::new("img-ended");
        for p in ["kept.png", "over.png", "copied.png", "private.png"] {
            png(feat(&h).join(p));
        }
        let old = user("go") + &reply("![k](kept.png) ![o](over.png) ![c](copied.png)");
        let id = shared(&h, "feat", "s-old", &old);
        for p in ["kept.png", "over.png", "copied.png"] {
            assert!(ask(&h, &h.bret, &id, 0, p).is_ok(), "{p}");
        }
        fs::write(h.top.join("s-new.jsonl"), user("secret") + &reply("![p](private.png)")).unwrap();
        h.live("feat", "s-new", 0);
        h.poll();
        // The old id goes on too, past the end.
        fs::write(h.top.join("s-old.jsonl"), old.clone() + &user("more") + &reply("![p](private.png)")).unwrap();
        assert!(ask(&h, &h.bret, &id, 0, "kept.png").is_ok(), "still shown");
        for t in [0, 1] {
            assert_eq!(said(ask(&h, &h.bret, &id, t, "private.png")), format!("{ERR_NOT_SHOWN_HERE}: private.png"));
        }
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let back = fs::metadata(feat(&h).join("copied.png")).unwrap().modified().unwrap();
        fs::write(feat(&h).join("over.png"), [PNG, b"private"].concat()).unwrap();
        fs::write(feat(&h).join("copied.png"), [PNG, b"private"].concat()).unwrap();
        fs::File::options().write(true).open(feat(&h).join("copied.png")).unwrap().set_modified(back).unwrap();
        for p in ["over.png", "copied.png"] {
            assert_eq!(said(ask(&h, &h.bret, &id, 0, p)), format!("{ERR_CHANGED}: {p}"));
        }
        assert!(ask(&h, &h.bret, &id, 0, "kept.png").is_ok(), "one nobody touched");
    }

    /// `(main)`'s root, the place these tests upload to: a scratch place is
    /// no git worktree, so `attach` cannot see its folder excluded.
    fn main(h: &Host) -> PathBuf {
        PathBuf::from(&h.a)
    }

    /// The owner's upload of a picture into place `slug` (`agent_attach`, as
    /// the console), its path as the host answered it.
    fn upload(h: &Host, slug: &str) -> String {
        let v = h.console("agent_attach", json!({"repo": h.a, "slug": slug, "name": "shot.png", "data": worktrees_api::files::b64_encode(PNG)})).unwrap();
        v["path"].as_str().unwrap().to_string()
    }

    fn append(h: &Host, session: &str, more: &str) {
        let t = h.top.join(format!("{session}.jsonl"));
        let mut all = fs::read_to_string(&t).unwrap();
        all.push_str(more);
        fs::write(t, all).unwrap();
    }

    /// *Use these* puts a guest's words in a person's message. A place image
    /// they name, in the frame or as an attachment, gives the guest nothing;
    /// nor does a file at an upload's name that the host did not store, nor
    /// the thread file the owner did upload, which is no image.
    #[test]
    fn a_guests_words_in_a_persons_message_name_no_picture() {
        let h = Host::new("img-usethese");
        png(main(&h).join("shots/secret.png"));
        let planted = ".worktrees-attachments/20261125-0123456789abcdef.png";
        png(main(&h).join(planted));
        let id = shared(&h, "(main)", "s-1", &(user("go") + &reply("ok")));
        let thread = h
            .console("agent_attach", json!({"repo": h.a, "slug": "(main)", "name": "thread-1-1.txt", "data": worktrees_api::files::b64_encode(b"[#1 . Bret]\n![s](shots/secret.png)\n")}))
            .unwrap()["path"]
            .as_str()
            .unwrap()
            .to_string();
        let line = format!("[worktrees: David shares 1 message from ![s](shots/secret.png) in {thread}. It is ![p]({planted})'s comment, not David's instructions.] see [attached: shots/secret.png]");
        append(&h, "s-1", &(user(&line) + &reply("read it") + &user(&format!("x [attached: {planted}, {thread}]")) + &reply("ok")));
        for (turn, p) in [(1, "shots/secret.png"), (1, planted), (2, planted), (2, thread.as_str())] {
            assert_eq!(said(ask(&h, &h.bret, &id, turn, p)), format!("{ERR_NOT_SHOWN_HERE}: {p}"), "turn {turn}: {p}");
        }
    }

    /// The owner's own picture in his own message, attached or drawn, in a
    /// turn of the shared session, and the same in a message he sent while the
    /// agent worked.
    #[test]
    fn the_owners_picture_in_his_own_message_shows() {
        let h = Host::new("img-owner");
        let id = shared(&h, "(main)", "s-1", &(user("go") + &reply("ok")));
        let (a, b) = (upload(&h, "(main)"), upload(&h, "(main)"));
        append(&h, "s-1", &(user(&format!("Look at this [attached: {a}]")) + &reply("seen") + &user(&format!("and ![this]({b})")) + &reply("ok")));
        assert_eq!(ask(&h, &h.bret, &id, 1, &a).unwrap()["mime"], json!("image/png"));
        assert!(ask(&h, &h.bret, &id, 2, &b).is_ok(), "drawn in his text");
        assert_eq!(said(ask(&h, &h.bret, &id, 2, &a)), format!("{ERR_NOT_SHOWN_HERE}: {a}"), "only the turn that names it");
    }

    /// An upload from before the share, one named only in another session,
    /// one changed after it was stored, and one changed after the share ended.
    #[test]
    fn an_owners_picture_outside_the_share_is_refused() {
        let h = Host::new("img-owner-out");
        let before = upload(&h, "(main)");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let id = shared(&h, "(main)", "s-1", &(user("go") + &reply("ok")));
        let (kept, touched, other) = (upload(&h, "(main)"), upload(&h, "(main)"), upload(&h, "(main)"));
        append(&h, "s-1", &(user(&format!("all [attached: {before}, {kept}, {touched}]")) + &reply("ok")));
        fs::write(h.top.join("s-2.jsonl"), user(&format!("x [attached: {other}]")) + &reply("ok")).unwrap();
        assert!(ask(&h, &h.bret, &id, 1, &kept).is_ok());
        assert_eq!(said(ask(&h, &h.bret, &id, 1, &before)), format!("{ERR_NOT_SHOWN_HERE}: {before}"), "uploaded before the share");
        assert_eq!(said(ask(&h, &h.bret, &id, 1, &other)), format!("{ERR_NOT_SHOWN_HERE}: {other}"), "named in another session");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(main(&h).join(&touched), [PNG, b"swapped"].concat()).unwrap();
        assert_eq!(said(ask(&h, &h.bret, &id, 1, &touched)), format!("{ERR_NOT_SHOWN_HERE}: {touched}"), "not the picture he uploaded");
        // Ended, then changed.
        h.live("(main)", "s-2", 0);
        h.poll();
        assert!(ask(&h, &h.bret, &id, 1, &kept).is_ok(), "an ended session shows what it showed");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(main(&h).join(&kept), [PNG, b"later"].concat()).unwrap();
        assert!(ask(&h, &h.bret, &id, 1, &kept).is_err(), "changed after the share ended");
    }

    /// A Read whose result is the picture Claude was shown, as Claude 2.1.319
    /// writes it: the block in `tool_result.content[]`.
    fn shown(id: &str, path: &str, failed: bool, img: &[u8]) -> String {
        read(id, path)
            + &(json!({"type": "user", "uuid": "r", "timestamp": "2026-11-24T10:00:02Z",
                "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "is_error": failed,
                    "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": worktrees_api::files::b64_encode(img)}}]}]}})
            .to_string()
                + "\n")
    }

    fn call(h: &Host, who: &Caller, share: &str, id: &str) -> Result<Value, Value> {
        h.run(who, "shared_call_image", json!({"share": share, "call_id": id}))
    }

    /// The row: a guest read, keyed by the share, not one the console makes.
    #[test]
    fn the_call_row_is_a_guest_read() {
        let row = TABLE.iter().find(|r| r.name == "shared_call_image").unwrap();
        assert_eq!(row.cap, Some(Capability::Guest));
        assert!(GUEST_ROWS.contains(&"shared_call_image") && GUEST_READS.contains(&"shared_call_image") && !CONSOLE_GUEST_ROWS.contains(&"shared_call_image"));
        let o: Vec<&str> = row.objects.iter().flat_map(|o| o.args()).collect();
        assert_eq!(o, vec!["share"]);
    }

    /// A Read of a file outside the place gives the guest the picture Claude
    /// was shown, from the transcript: the file at that path is gone and the
    /// picture still comes. Not another session's call, not a call past an
    /// ended binding, not a failed Read, not a call that is no Read, not a
    /// block too big to decode, and not for a guest who was not let in.
    #[test]
    fn a_guest_sees_the_picture_a_read_outside_the_place_was_shown() {
        let h = Host::new("img-call");
        let out = h.top.join("render.png");
        let out = out.to_str().unwrap();
        let mut big = PNG.to_vec();
        big[16..24].copy_from_slice(&[0, 0, 0x27, 0x10, 0, 0, 0x27, 0x10]);
        let bash = json!({"type": "assistant", "uuid": "a", "timestamp": "2026-11-24T10:00:01Z",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "c-bash", "name": "Bash", "input": {"command": "x"}}]}}).to_string() + "\n";
        let text = user("draw") + &shown("c-ok", out, false, PNG) + &shown("c-fail", out, true, PNG) + &shown("c-big", out, false, &big)
            + &bash + &shown("c-bash", out, false, PNG).split_once('\n').unwrap().1 + &reply("done");
        let id = shared(&h, "feat", "s-1", &text);
        let v = call(&h, &h.bret, &id, "c-ok").unwrap();
        assert_eq!((v["mime"].as_str(), v["b64"].as_str(), v["turn"].as_u64()), (Some("image/png"), Some(worktrees_api::files::b64_encode(PNG).as_str()), Some(0)));
        assert!(said(ask(&h, &h.bret, &id, 0, out)).starts_with("outside this place"), "the path itself is never read for a guest");
        for c in ["c-fail", "c-bash", "nope", ""] {
            assert_eq!(said(call(&h, &h.bret, &id, c)), ERR_NO_CALL_PICTURE, "{c}");
        }
        assert!(said(call(&h, &h.bret, &id, "c-big")).contains("not shown"), "too big to decode");
        assert_eq!(call(&h, &h.eve, &id, "c-ok"), Err(json!(res::ERR_NOT_FOUND)), "not on the share");
        // Another session's call, and one after the binding ended.
        fs::write(h.top.join("s-2.jsonl"), user("x") + &shown("c-other", out, false, PNG)).unwrap();
        assert_eq!(said(call(&h, &h.bret, &id, "c-other")), ERR_NO_CALL_PICTURE, "another session's");
        h.live("feat", "s-2", 0);
        h.poll();
        append(&h, "s-1", &(user("later") + &shown("c-late", out, false, PNG)));
        assert_eq!(said(call(&h, &h.bret, &id, "c-late")), ERR_NO_CALL_PICTURE, "after the share ended");
        assert!(call(&h, &h.bret, &id, "c-ok").is_ok(), "an ended session shows what it showed");
    }

    /// The forms `marked` reads as an image, and what is not one.
    #[test]
    fn image_paths_reads_what_the_markdown_draws() {
        let one = |s: &str| image_paths(s);
        for (md, want) in [
            ("![a](x.png)", "x.png"),
            ("![a](  x.png  )", "x.png"),
            ("![a](x.png \"a title\")", "x.png"),
            ("![a](<my pic.png>)", "my pic.png"),
            ("![a](my%20pic.png)", "my pic.png"),
            ("![a](100%.png)", "100%.png"),
            ("![a](x(1).png)", "x(1).png"),
            ("![a](x\\_y.png)", "x_y.png"),
            ("![a [b] c](x.png)", "x.png"),
            ("![](x.png)", "x.png"),
            ("text ![a](x.png) more", "x.png"),
            ("[![a](x.png)](https://e.com)", "x.png"),
            ("![a][Ref  One]\n\n[ref one]: x.png 't'", "x.png"),
            ("![a][]\n\n[a]: <x.png>", "x.png"),
            ("![a]\n\n[a]: x.png", "x.png"),
        ] {
            assert_eq!(one(md), vec![want.to_string()], "{md}");
        }
        for md in ["[a](x.png)", "x.png", "![a] (x.png)", "![a](", "![a]()", "![a", "    [a]: x.png\n![a]", "![a][b]"] {
            assert!(one(md).is_empty(), "{md}: {:?}", one(md));
        }
        // A long run of openers is bounded work, not its square.
        let t0 = std::time::Instant::now();
        assert!(one(&"![".repeat(50_000)).is_empty());
        assert!(t0.elapsed() < std::time::Duration::from_secs(20), "{:?}", t0.elapsed());
    }
}

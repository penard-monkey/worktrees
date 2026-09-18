//! How a place is NAMED inside a Claude session — the `@worktrees:place://…`
//! token the `@` menu completes and the nav drag inserts.
//!
//! This lives in core, and not in the MCP server that serves the resources,
//! because there are now TWO producers of the same token: `worktrees mcp`
//! (`resources/list`, which the client caches and matches a mention against by
//! exact string) and the app's nav drag. The client resolves a mention with
//! `find(r => r.uri === typed)` — first match wins, silently — so a second
//! implementation that drifted by one character would not error, it would
//! quietly reference the wrong place or nothing at all.
//!
//! The app therefore never builds this string: it asks the backend for the
//! finished token. That is deliberately the opposite of `dnd.ts::predictTier`,
//! which mirrors a core rule in TypeScript and needs `dnd-check.mjs` to catch
//! the drift; here there is nothing to mirror.

use std::path::Path;

use crate::model::PlaceRef;

/// The MCP server name a profile launch registers, verbatim (`profile.rs`
/// inserts the stanza under this key, and a profile launch adds
/// `--strict-mcp-config`, which drops any user-scope entry).
///
/// The SAME constant the setup wizard installs under — one name, one place.
pub const DEFAULT_SERVER: &str = crate::mcpsetup::SERVER_KEY;

/// Turn a slug into something that survives Claude Code's TWO @-mention rules.
///
/// `ops::slugify` is `s.replace('/', "-")` and nothing else, so every git-legal
/// branch character reaches a slug — and the client applies two different,
/// narrower filters to a mention:
///
///   * the menu's token charset, `/^@[\p{L}\p{N}\p{M}_\-./\\()[\]~:]*/u`, so a
///     uri containing `@ # % + = , ! &` cannot be typed-to-complete at all
///     (note `%` is excluded — percent-encoding is NOT a way out); and
///   * the submit-time extractor, `/…@([^\s]+:[^\s]+)\b/g`, whose trailing `\b`
///     drops any uri that does not END on a word character.
///
/// `(` and `)` pass the first and fail the second, which is the worst case:
/// `place://(main)` completes in the menu and then silently resolves to
/// nothing. So the output here is deliberately narrower than either rule.
///
/// Two known edges, both requiring a deliberately strange branch name and
/// neither fixable without a general-category table: enclosed alphanumerics
/// (`Ⓐ`) satisfy `is_alphanumeric` but not the menu's `\p{L}\p{N}\p{M}`, and a
/// combining mark is the reverse, so a decomposed `café` folds to `cafe`.
pub fn safe_uri_part(slug: &str) -> String {
    let mut out = String::with_capacity(slug.len());
    for c in slug.chars() {
        // `is_alphanumeric`, not `is_ascii_alphanumeric`: both client rules
        // admit a Cyrillic or CJK slug MID-uri, and folding those to `-` turned
        // every non-ASCII place into `place`, `place-2`, … — names that
        // identify nothing and renumber whenever a sibling appears.
        if c.is_alphanumeric() || c == '_' || c == '.' || c == '-' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches(|c: char| c == '-' || c == '.');
    if trimmed.is_empty() {
        return "place".to_string();
    }
    // The LAST character is the one rule that stays ASCII: JavaScript's `\b` is
    // ASCII even under the `u` flag, so a uri ending in a non-ASCII letter fails
    // the submit extractor exactly the way `wip-` does. `_` is a word character
    // in both of the client's charsets.
    match trimmed.chars().next_back() {
        Some(c) if c.is_ascii_alphanumeric() || c == '_' => trimmed.to_string(),
        _ => format!("{trimmed}_"),
    }
}

/// `place://…` uris for a whole index, deduplicated.
///
/// Two slugs can sanitise to one uri (`feat/x` and `feat-x` both give
/// `feat-x`), and the client takes the FIRST match, so collisions are broken
/// here, in the index's own order (main first, then glob order).
///
/// Pass one reserves every slug that needs no sanitising, because suffixing
/// naively re-created the bug this exists to close: dirs `wip`, `wip-`, `wip-2`
/// mapped to `wip`, `wip-2`, `wip-2-2`, so `place://wip-2` named the dir `wip-`
/// while the dir actually called `wip-2` answered to something else. It was
/// unstable too — creating `wip` renumbered `wip-`.
pub fn uri_map(places: &[PlaceRef]) -> Vec<(String, &PlaceRef)> {
    let mut used: std::collections::HashSet<String> = places
        .iter()
        .filter(|p| safe_uri_part(&p.slug) == p.slug)
        .map(|p| format!("place://{}", p.slug))
        .collect();
    let mut out = Vec::with_capacity(places.len());
    for p in places {
        let base = safe_uri_part(&p.slug);
        let clean = format!("place://{base}");
        if base == p.slug {
            // Reserved above. Two places cannot share a slug, so this is unique.
            out.push((clean, p));
            continue;
        }
        let mut uri = clean;
        let mut n = 2;
        while !used.insert(uri.clone()) {
            uri = format!("place://{base}-{n}");
            n += 1;
        }
        out.push((uri, p));
    }
    out
}

/// The uri for one slug within its own index, or `None` if that slug is not in
/// it. Resolved against the WHOLE index because a uri depends on its
/// neighbours: the same slug can be `place://main` or `place://main-2`
/// depending on who else exists.
pub fn uri_for(places: &[PlaceRef], slug: &str) -> Option<String> {
    uri_map(places).into_iter().find(|(_, p)| p.slug == slug).map(|(u, _)| u)
}

/// The name the worktrees MCP server is registered under in this user's
/// `~/.claude.json`, if it is there.
///
/// Delegates to `mcpsetup::our_key`, which is the judgement the setup wizard's
/// status and install already use — "is this stanza ours" is one question and
/// must not have two answers. A profile launch does not consult this at all: it
/// writes its own stanza under [`DEFAULT_SERVER`] and passes
/// `--strict-mcp-config`, which drops user scope entirely.
pub fn server_name_in(user_claude_json: &serde_json::Value) -> Option<String> {
    crate::mcpsetup::our_key(user_claude_json)
}

/// The server name the session running in `into_slug` will actually have
/// registered — which is not always what `~/.claude.json` says.
///
/// A PROFILE launch writes its own stanza under [`DEFAULT_SERVER`] and passes
/// `--strict-mcp-config`, which drops the user-scope entry entirely
/// (`profile.rs`). So for a profiled session the user's own naming is
/// irrelevant, and consulting it would produce `@wt:…` for a session that only
/// answers to `@worktrees:…` — a token that completes nowhere and expands to
/// nothing. `Declared::profile_id` is the launch stamp that says which it was.
pub fn server_name_for(repo: &str, into_slug: &str, user_claude_json: &Path) -> String {
    let profiled = crate::store::read_lenient(repo)
        .places
        .get(into_slug)
        .and_then(|d| d.profile_id.as_ref())
        .is_some();
    if profiled {
        return DEFAULT_SERVER.to_string();
    }
    std::fs::read(user_claude_json)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .as_ref()
        .and_then(server_name_in)
        .unwrap_or_else(|| DEFAULT_SERVER.to_string())
}

/// The whole token to type, e.g. `@worktrees:place://bug-fixes`.
///
/// Leading and trailing spaces are the CALLER's job — the submit extractor
/// requires whitespace (or start-of-input) before the `@`, and the app cannot
/// see the input buffer to know whether there already is any.
pub fn mention(server: &str, uri: &str) -> String {
    format!("@{server}:{uri}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(slug: &str, is_main: bool) -> PlaceRef {
        PlaceRef {
            slug: slug.to_string(),
            path: format!("/tmp/{slug}"),
            branch: None,
            registered: true,
            is_main,
        }
    }

    #[test]
    fn a_uri_part_survives_both_of_the_clients_mention_rules() {
        assert_eq!(safe_uri_part("(main)"), "main", "parens must not reach a uri");
        assert_eq!(safe_uri_part("bug-fixes"), "bug-fixes");
        assert_eq!(safe_uri_part("feat/thing"), "feat-thing", "a slash is legal in a slug");
        assert_eq!(safe_uri_part("v1.2"), "v1.2", "a dot is fine mid-uri");
        assert_eq!(safe_uri_part("wip-"), "wip", "a trailing dash would fail \\b");
        assert_eq!(safe_uri_part("x."), "x", "so would a trailing dot");
        assert_eq!(safe_uri_part("a+b=c"), "a-b-c", "chars the MENU cannot type");
        assert_eq!(safe_uri_part("50%"), "50", "percent-encoding is not available either");
        assert_eq!(safe_uri_part("!!!"), "place", "never empty");
        assert_eq!(safe_uri_part("\u{444}\u{443}\u{43d}"), "\u{444}\u{443}\u{43d}_", "a Cyrillic slug keeps its name");
        assert_eq!(safe_uri_part("caf\u{e9}-x"), "caf\u{e9}-x", "\u{2026}and only pays for the trailing rule");
        for s in ["(main)", "feat/x", "a+b", "wip-", "50%", "!!!", "\u{444}\u{443}\u{43d}", "\u{65e5}\u{672c}\u{8a9e}"] {
            let u = safe_uri_part(s);
            assert!(
                u.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-'),
                "{s} -> {u} left a character outside the safe set"
            );
            let last = u.chars().last().unwrap();
            assert!(last.is_ascii_alphanumeric() || last == '_', "{s} -> {u} ends on a non-word char");
        }
    }

    #[test]
    fn colliding_slugs_get_distinct_uris_and_a_clean_slug_keeps_its_name() {
        let places = vec![p("(main)", true), p("main", false), p("feat/x", false), p("feat-x", false)];
        let got: Vec<String> = uri_map(&places).into_iter().map(|(u, _)| u).collect();
        assert_eq!(
            got,
            vec!["place://main-2", "place://main", "place://feat-x-2", "place://feat-x"],
            "a slug that needs no sanitising keeps its own name; the dirty one moves"
        );

        // The regression that made the two-pass reservation necessary.
        let shadow = vec![p("wip", false), p("wip-", false), p("wip-2", false)];
        let pairs: Vec<(String, String)> =
            uri_map(&shadow).into_iter().map(|(u, r)| (u, r.slug.clone())).collect();
        for (uri, slug) in &pairs {
            if let Some(bare) = uri.strip_prefix("place://") {
                assert!(
                    bare == slug || shadow.iter().all(|p| p.slug != *bare),
                    "{uri} reads as the dir {bare:?} but resolves to {slug:?}"
                );
            }
        }
        for got in [
            uri_map(&places).into_iter().map(|(u, _)| u).collect::<Vec<_>>(),
            pairs.iter().map(|(u, _)| u.clone()).collect::<Vec<_>>(),
        ] {
            let uniq: std::collections::HashSet<&String> = got.iter().collect();
            assert_eq!(uniq.len(), got.len(), "uris must be unique: {got:?}");
        }
    }

    /// The drag and `resources/list` must produce the SAME string for the same
    /// place — the client matches a mention by exact equality, so a one-character
    /// difference is a silent miss, not an error.
    #[test]
    fn uri_for_agrees_with_the_served_list() {
        let places = vec![p("(main)", true), p("main", false), p("feat/x", false)];
        for (served, r) in uri_map(&places) {
            assert_eq!(
                uri_for(&places, &r.slug).as_deref(),
                Some(served.as_str()),
                "the drag's uri for {:?} diverged from the one the server serves",
                r.slug
            );
        }
        assert_eq!(uri_for(&places, "nope"), None);
    }

    #[test]
    fn the_server_name_is_found_by_command_not_by_key() {
        let j = serde_json::json!({ "mcpServers": {
            "other": { "command": "/usr/bin/something", "args": ["mcp"] },
            "wt":    { "command": "/Users/x/.local/bin/worktrees", "args": ["mcp", "--mutations"] },
        }});
        assert_eq!(server_name_in(&j).as_deref(), Some("wt"), "a renamed server is still ours");

        // `worktrees` the binary, but not the MCP subcommand.
        let not_mcp = serde_json::json!({ "mcpServers": {
            "x": { "command": "/usr/local/bin/worktrees", "args": ["ls"] },
        }});
        assert_eq!(server_name_in(&not_mcp), None);
        assert_eq!(server_name_in(&serde_json::json!({})), None);
        assert_eq!(server_name_in(&serde_json::Value::Null), None);
    }

    /// A profiled session answers only to `worktrees`, whatever the user called
    /// their own user-scope server — `--strict-mcp-config` drops that entry.
    #[test]
    fn a_profiled_session_ignores_the_users_own_server_name() {
        let dir = std::env::temp_dir().join(format!("wt-mention-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = dir.to_string_lossy().to_string();
        let user = dir.join("claude.json");
        std::fs::write(
            &user,
            serde_json::json!({ "mcpServers": {
                "wt": { "command": "/Users/x/.local/bin/worktrees", "args": ["mcp"] }
            }})
            .to_string(),
        )
        .unwrap();

        // Unprofiled: the user's own name is what that session registered.
        assert_eq!(server_name_for(&repo, "plain", &user), "wt");

        // Profiled: the stamp wins.
        crate::store::edit(&repo, "profiled", |d| d.profile_id = Some("p1".into())).unwrap();
        assert_eq!(server_name_for(&repo, "profiled", &user), DEFAULT_SERVER);

        // No `~/.claude.json` at all is the common case, not an error.
        assert_eq!(server_name_for(&repo, "plain", &dir.join("nope.json")), DEFAULT_SERVER);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mention_is_shaped_the_way_the_client_parses_it() {
        let m = mention(DEFAULT_SERVER, "place://bug-fixes");
        assert_eq!(m, "@worktrees:place://bug-fixes");
        // The client splits on the FIRST colon: server, then the rest as uri.
        let (server, uri) = m[1..].split_once(':').expect("a mention carries a colon");
        assert_eq!(server, "worktrees");
        assert_eq!(uri, "place://bug-fixes");
        assert!(!m.contains(char::is_whitespace), "whitespace would end the token");
    }
}

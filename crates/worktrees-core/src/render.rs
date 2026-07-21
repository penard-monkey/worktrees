//! ANSI + the human `ls` table renderer. Byte-for-byte parity with the bash
//! `cmd_ls` is the contract (ls.bats asserts absolute column offsets), so the
//! format strings, spacing, color-outside-padding, and `fit()` truncation here
//! mirror bin/worktrees exactly. Real ESC bytes (bash interpolates `\033` via
//! printf `%b`/`echo -e`; we emit the same output bytes directly).

pub const RED: &str = "\x1b[0;31m";
pub const GREEN: &str = "\x1b[0;32m";
pub const YELLOW: &str = "\x1b[1;33m";
pub const CYAN: &str = "\x1b[0;36m";
pub const DIM: &str = "\x1b[2m";
pub const NC: &str = "\x1b[0m";

/// `info` line, e.g. the empty-`.worktrees/` message. Matches `info()` (stdout).
pub fn info_line(msg: &str) -> String {
    format!("{GREEN}▸{NC} {msg}\n")
}

/// `error` line (stderr), matches `error()`.
pub fn error_line(msg: &str) -> String {
    format!("{RED}✗{NC} {msg}")
}

/// Truncate to width `w` chars, marking a cut with `~` (bash `fit`).
pub fn fit(s: &str, w: usize) -> String {
    if s.chars().count() > w {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('~');
        t
    } else {
        s.to_string()
    }
}

/// Left-justify to `w` chars (space pad; no truncation — like printf `%-*s`).
fn pad(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n >= w {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(w - n))
    }
}

/// One rendered row's plain fields (color applied here, outside the padding).
pub struct Row {
    pub slug: String,
    pub btext: String,
    pub bcol: &'static str, // color for the branch cell, or ""
    pub created: String,
    pub age: String,
    pub tmux_cell: String, // already colored (glyph)
    pub git_cell: String,  // already colored
    pub key: i64,
}

/// Render the full human `ls` table (header + recency-sorted rows + footer),
/// returning the exact stdout bytes. `rows` are in glob (name-sorted) order;
/// we stable-sort by `key` descending so ties keep glob order (bash `sort -s`).
pub fn table(mut rows: Vec<Row>) -> String {
    let mut maxslug = 4usize; // "SLUG"
    let mut maxbranch = 6usize; // "BRANCH"
    for r in &rows {
        maxslug = maxslug.max(r.slug.chars().count());
        maxbranch = maxbranch.max(r.btext.chars().count());
    }
    maxslug = maxslug.min(44);
    maxbranch = maxbranch.min(44);

    // stable sort by key desc (glob order preserved on ties)
    rows.sort_by(|a, b| b.key.cmp(&a.key));

    let mut out = String::new();
    // header: SLUG 2 BRANCH 2 CREATED(10) 2 COMMIT(5) 2 TMUX(4) 1 GIT
    out.push_str(&format!(
        "{DIM}{}  {}  {}  {}  {} {}{NC}\n",
        pad("SLUG", maxslug),
        pad("BRANCH", maxbranch),
        pad("CREATED", 10),
        pad("COMMIT", 5),
        pad("TMUX", 4),
        "GIT",
    ));
    for r in &rows {
        // branch cell: pad plain to maxbranch, THEN wrap color (escape codes must
        // not count toward width).
        let mut bpad = pad(&fit(&r.btext, maxbranch), maxbranch);
        if !r.bcol.is_empty() {
            bpad = format!("{}{}{NC}", r.bcol, bpad);
        }
        // row: slug 2 bpad 2 created(10) 2 age(5) 2 tmux 3 git   (note the 3-space
        // gap before GIT — the deliberate header/row asymmetry ls.bats pins).
        out.push_str(&format!(
            "{}  {}  {}  {}  {}   {}\n",
            pad(&fit(&r.slug, maxslug), maxslug),
            bpad,
            pad(&r.created, 10),
            pad(&r.age, 5),
            r.tmux_cell,
            r.git_cell,
        ));
    }
    out.push_str(&format!(
        "{DIM}● session live · {CYAN}branch{NC}{DIM} ≠ name · COMMIT = age of last commit.  open <slug> · switch <slug> <branch> · rm <slug>{NC}\n"
    ));
    out
}

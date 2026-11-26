//! Who started which place, and where it hangs (`docs/proposals/lane-children.md`).
//!
//! Three declared links per place (`store::Declared`): `started_by` (origin,
//! written once at creation), `handed_to` (written when the starter is
//! removed) and `under` (adoption, display only). This module is the ONE
//! reading of them. It is pure: it takes the places that exist, their birth
//! times and their declared state, and a liveness question it asks lazily,
//! and answers per place:
//!
//! - `starter`: `started_by`, with whether the place it names is still there;
//! - `parent`: where the place is DRAWN — the first link that holds of
//!   `under`, `handed_to`, `started_by`; absent means top level;
//! - `reports_to`: where a `report` with no `to` goes (§6) — up `handed_to`,
//!   else `started_by`, to the first place with an agent running, else
//!   `(main)`. `under` never moves a report.
//!
//! A link HOLDS only when (§2):
//! 1. the place it names exists, or is `(main)`;
//! 2. that place was not born after the link was written (a removed slug
//!    made again is a different place) — a birth time the platform cannot
//!    give passes;
//! 3. following links upward from it never comes back to the place itself
//!    — or, when it does (a loop only a hand edit can make), it is not the
//!    newest link in that loop. A tie fails them all.
//!
//! A link that fails is read as absent. Nothing here rewrites the store.
//! A link into another project (`started_by.project`) is shown as the
//! starter and never nests or routes (§3, §6 K).
//!
//! Labels, never grants: nothing may read these to decide what a caller can do.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::store::{Declared, Link};

/// The main checkout's slug, and the top of every walk.
pub const MAIN: &str = "(main)";

/// `lineage.starter`: the recorded starter, as the header names it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Starter {
    pub place: String,
    pub at: i64,
    /// Set when the starter is in another registered project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The place it names is gone (removed, or its slug now belongs to a
    /// place made later). Never set for another project's starter, which this
    /// snapshot cannot see.
    #[serde(default, skip_serializing_if = "is_false")]
    pub removed: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// What a place's snapshot row carries as `lineage`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Lineage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starter: Option<Starter>,
    /// `under`, when it holds: who moved it, where, and when.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under: Option<Link>,
    /// `handed_to`, when it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handed_to: Option<Link>,
    /// The drawn parent's slug. Absent: top level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The next place up by `handed_to`, else `started_by`: where a removal
    /// of this place moves its `lanes`, and the first step of the report
    /// walk. Absent: `(main)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<String>,
    /// Where a `report` with no `to` goes now. Absent only for `(main)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reports_to: Option<String>,
    /// The places whose starter (holding) is this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub started: Vec<String>,
    /// The places that hang from this one by `handed_to` or `started_by`:
    /// exactly the ones a removal of this place moves up (§7).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lanes: Vec<String>,
}

/// Where a default report goes, and the places on the way that were skipped
/// because no agent runs there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub to: String,
    pub skipped: Vec<String>,
}

/// The places of one project, as the rules see them.
pub struct Index<'a> {
    /// slug -> (birth epoch, declared)
    nodes: BTreeMap<&'a str, (Option<i64>, Option<&'a Declared>)>,
}

impl<'a> Index<'a> {
    /// `places`: every place that exists (the snapshot's slugs, `(main)`
    /// included or not) with its birth epoch; `decl`: the store's places.
    pub fn new(places: impl IntoIterator<Item = (&'a str, Option<i64>)>, decl: &'a BTreeMap<String, Declared>) -> Index<'a> {
        let nodes = places.into_iter().map(|(s, born)| (s, (born, decl.get(s)))).collect();
        Index { nodes }
    }

    fn decl(&self, slug: &str) -> Option<&'a Declared> {
        self.nodes.get(slug).and_then(|(_, d)| *d)
    }

    pub fn exists(&self, slug: &str) -> bool {
        slug == MAIN || self.nodes.contains_key(slug)
    }

    /// Rules 1 and 2 for a link of `slug`'s.
    fn names_live_place(&self, slug: &str, l: &Link) -> bool {
        if l.project.is_some() || l.place == slug {
            return false;
        }
        if l.place == MAIN {
            return true;
        }
        match self.nodes.get(l.place.as_str()) {
            None => false,
            Some((born, _)) => born.is_none_or(|b| b <= 0 || b <= l.at),
        }
    }

    /// The first of `links` that holds rules 1 and 2 — no cycle check.
    fn first_raw(&self, slug: &str, links: &[Option<&'a Link>]) -> Option<&'a Link> {
        links.iter().flatten().copied().find(|l| self.names_live_place(slug, l))
    }

    /// The first of `links` that holds all three rules, with `chain` the raw
    /// step the cycle check walks.
    fn first_holding(&self, slug: &str, links: &[Option<&'a Link>], chain: fn(&Self, &str) -> Option<&'a Link>) -> Option<&'a Link> {
        links.iter().flatten().copied().filter(|l| self.names_live_place(slug, l)).find(|l| {
            // Rule 3: walk the RAW chain up from the candidate. When it comes
            // back to `slug`, the loop is a hand edit's (every writer refuses
            // one), and the link read as absent is the NEWEST in it — the one
            // that closed it — so an adoption never outvotes the recorded
            // origin. A tie fails every link in it. A loop elsewhere just
            // stops the walk.
            let mut seen = Seen::new();
            let mut cur = l.place.clone();
            let mut others: Vec<i64> = Vec::new();
            while cur != MAIN && seen.insert(cur.clone()) {
                if cur == slug {
                    return others.iter().any(|at| *at > l.at);
                }
                match chain(self, &cur) {
                    Some(next) => {
                        others.push(next.at);
                        cur = next.place.clone();
                    }
                    None => break,
                }
            }
            true
        })
    }

    fn draw_links(&self, slug: &str) -> [Option<&'a Link>; 3] {
        let d = self.decl(slug);
        [d.and_then(|d| d.under.as_ref()), d.and_then(|d| d.handed_to.as_ref()), d.and_then(|d| d.started_by.as_ref())]
    }

    fn up_links(&self, slug: &str) -> [Option<&'a Link>; 2] {
        let d = self.decl(slug);
        [d.and_then(|d| d.handed_to.as_ref()), d.and_then(|d| d.started_by.as_ref())]
    }

    fn raw_draw(&self, slug: &str) -> Option<&'a Link> {
        self.first_raw(slug, &self.draw_links(slug))
    }

    fn raw_up(&self, slug: &str) -> Option<&'a Link> {
        self.first_raw(slug, &self.up_links(slug))
    }

    /// The drawn parent's slug; `None` for top level.
    pub fn parent(&self, slug: &str) -> Option<String> {
        self.first_holding(slug, &self.draw_links(slug), Self::raw_draw).map(|l| l.place.clone()).filter(|p| p != MAIN)
    }

    /// The next place up for reports (`handed_to`, else `started_by`);
    /// `None` for `(main)` / top.
    pub fn up(&self, slug: &str) -> Option<String> {
        self.first_holding(slug, &self.up_links(slug), Self::raw_up).map(|l| l.place.clone()).filter(|p| p != MAIN)
    }

    /// §6 rules 3 and 4: the nearest ancestor up the report chain that
    /// `live` says has an agent running, else `(main)`.
    pub fn route(&self, slug: &str, live: &dyn Fn(&str) -> bool) -> Route {
        let mut skipped = Vec::new();
        let mut seen = Seen::new();
        let mut cur = slug.to_string();
        seen.insert(cur.clone());
        while let Some(next) = self.up(&cur) {
            if !seen.insert(next.clone()) {
                break;
            }
            if live(&next) {
                return Route { to: next, skipped };
            }
            skipped.push(next.clone());
            cur = next;
        }
        Route { to: MAIN.to_string(), skipped }
    }

    /// `slug`'s whole lineage. `live` is asked only for places on its report
    /// chain.
    pub fn lineage(&self, slug: &str, live: &dyn Fn(&str) -> bool) -> Lineage {
        let d = self.decl(slug);
        let starter = d.and_then(|d| d.started_by.as_ref()).filter(|l| l.place != slug).map(|l| Starter {
            place: l.place.clone(),
            at: l.at,
            project: l.project.clone(),
            removed: l.project.is_none() && !self.names_live_place(slug, l),
        });
        let holding = |l: Option<&'a Link>| l.filter(|l| self.names_live_place(slug, l)).cloned();
        let others = || self.nodes.keys().filter(|s| **s != slug);
        Lineage {
            starter,
            under: holding(d.and_then(|d| d.under.as_ref())),
            handed_to: holding(d.and_then(|d| d.handed_to.as_ref())),
            parent: self.parent(slug),
            up: self.up(slug),
            reports_to: (slug != MAIN).then(|| self.route(slug, live).to),
            started: others()
                .filter(|s| {
                    self.decl(s)
                        .and_then(|d| d.started_by.as_ref())
                        .is_some_and(|l| l.place == slug && self.names_live_place(s, l))
                })
                .map(|s| s.to_string())
                .collect(),
            lanes: others().filter(|s| self.up(s).as_deref() == Some(slug)).map(|s| s.to_string()).collect(),
        }
    }

    /// Every place's lineage, asking `live` at most once per place.
    pub fn resolve_all(&self, live: &dyn Fn(&str) -> bool) -> HashMap<String, Lineage> {
        let memo: RefCell<HashMap<String, bool>> = RefCell::new(HashMap::new());
        let cached = |s: &str| -> bool {
            if let Some(v) = memo.borrow().get(s) {
                return *v;
            }
            let v = live(s);
            memo.borrow_mut().insert(s.to_string(), v);
            v
        };
        self.nodes.keys().map(|s| (s.to_string(), self.lineage(s, &cached))).collect()
    }

    /// What removing `slug` does (§7): the lanes that hang from it and the
    /// place they move up to.
    pub fn handoff(&self, slug: &str) -> (Vec<String>, String) {
        let lanes = self.nodes.keys().filter(|s| **s != slug && self.up(s).as_deref() == Some(slug)).map(|s| s.to_string()).collect();
        (lanes, self.up(slug).unwrap_or_else(|| MAIN.to_string()))
    }

    /// Whether `slug` may be drawn under `target` (`(main)` = top level):
    /// both exist, `slug` is not `(main)`, and the move makes no cycle.
    pub fn check_under(&self, slug: &str, target: &str) -> Result<(), String> {
        if slug == MAIN {
            return Err("(main) is the top of every project; it cannot be moved".into());
        }
        if !self.nodes.contains_key(slug) {
            return Err(format!("no such place: {slug}"));
        }
        if !self.exists(target) {
            return Err(format!("no such place to move it under: {target}"));
        }
        if target == slug {
            return Err(format!("{slug} cannot be drawn under itself"));
        }
        // The target's drawn chain must not pass through `slug`, or the move
        // closes a cycle (§8: refused at write time).
        let mut seen = Seen::new();
        let mut cur = Some(target.to_string());
        while let Some(c) = cur {
            if c == slug {
                return Err(format!("{target} is drawn under {slug}; moving {slug} under it would make a cycle"));
            }
            if !seen.insert(c.clone()) {
                break;
            }
            cur = self.parent(&c);
        }
        Ok(())
    }
}

/// A set used by walks to stop on a loop a hand edit made.
type Seen = HashSet<String>;

#[cfg(test)]
mod tests {
    use super::*;

    fn link(place: &str, at: i64) -> Option<Link> {
        Some(Link::to(place, at))
    }

    struct World {
        born: Vec<(String, Option<i64>)>,
        decl: BTreeMap<String, Declared>,
    }

    impl World {
        fn new() -> World {
            World { born: vec![(MAIN.into(), Some(1))], decl: BTreeMap::new() }
        }
        fn place(mut self, slug: &str, born: i64) -> World {
            self.born.push((slug.into(), Some(born)));
            self
        }
        fn started(mut self, slug: &str, by: &str, at: i64) -> World {
            self.decl.entry(slug.into()).or_default().started_by = link(by, at);
            self
        }
        fn handed(mut self, slug: &str, to: &str, at: i64, from: &str) -> World {
            let mut l = Link::to(to, at);
            l.from = Some(from.into());
            self.decl.entry(slug.into()).or_default().handed_to = Some(l);
            self
        }
        fn under(mut self, slug: &str, to: &str, at: i64) -> World {
            self.decl.entry(slug.into()).or_default().under = link(to, at);
            self
        }
        fn index(&self) -> Index<'_> {
            Index::new(self.born.iter().map(|(s, b)| (s.as_str(), *b)), &self.decl)
        }
    }

    fn nobody(_: &str) -> bool {
        false
    }

    /// A family: A started B, B started C.
    fn family() -> World {
        World::new().place("a", 10).place("b", 20).place("c", 30).started("b", "a", 15).started("c", "b", 25)
    }

    #[test]
    fn a_holding_starter_is_the_parent_at_any_depth() {
        let w = family();
        let i = w.index();
        assert_eq!(i.parent("a"), None);
        assert_eq!(i.parent("b").as_deref(), Some("a"));
        assert_eq!(i.parent("c").as_deref(), Some("b"));
        assert_eq!(i.lineage("c", &nobody).starter.map(|s| s.place), Some("b".into()));
        assert_eq!(i.lineage("a", &nobody).started, vec!["b".to_string()]);
        assert_eq!(i.lineage("a", &nobody).lanes, vec!["b".to_string()]);
    }

    #[test]
    fn a_place_with_no_record_is_top_level_and_reports_to_main() {
        let w = World::new().place("old", 10);
        let i = w.index();
        let l = i.lineage("old", &|_| true);
        assert_eq!(l, Lineage { reports_to: Some(MAIN.into()), ..Default::default() });
        assert_eq!(i.lineage(MAIN, &nobody).reports_to, None, "(main) reports to nobody by default");
    }

    #[test]
    fn rule_1_a_starter_that_no_longer_exists_does_not_hold() {
        let w = World::new().place("c", 30).started("c", "gone", 25);
        let i = w.index();
        assert_eq!(i.parent("c"), None);
        let s = i.lineage("c", &nobody).starter.expect("still named");
        assert!(s.removed, "the header says (removed)");
        assert_eq!(i.route("c", &|_| true).to, MAIN);
    }

    #[test]
    fn rule_2_a_slug_made_again_after_the_link_is_a_different_place() {
        // `a` was removed and made again at 50; the link was written at 15.
        let w = World::new().place("a", 50).place("b", 20).started("b", "a", 15);
        let i = w.index();
        assert_eq!(i.parent("b"), None);
        assert!(i.lineage("b", &nobody).starter.unwrap().removed);
        assert_eq!(i.route("b", &|_| true).to, MAIN);
        assert!(i.lineage("a", &nobody).started.is_empty(), "the new `a` started nothing");
    }

    #[test]
    fn rule_2_passes_when_the_platform_gives_no_birth_time() {
        let mut w = World::new().place("b", 20).started("b", "a", 15);
        w.born.push(("a".into(), None));
        assert_eq!(w.index().parent("b").as_deref(), Some("a"));
    }

    #[test]
    fn rule_3_a_cycle_a_hand_edit_made_is_read_as_absent() {
        let w = World::new().place("a", 10).place("b", 20).under("a", "b", 30).under("b", "a", 30);
        let i = w.index();
        assert_eq!(i.parent("a"), None);
        assert_eq!(i.parent("b"), None);
        // A self-link never holds either.
        let w = World::new().place("a", 10).started("a", "a", 20);
        assert_eq!(w.index().parent("a"), None);
        assert_eq!(w.index().lineage("a", &nobody).starter, None);
    }

    #[test]
    fn a_cycle_falls_back_to_the_next_link() {
        // b started by a; a hand edit put a under b. a's `under` would close
        // a loop, so a stays top level, and b stays under a.
        let w = World::new().place("a", 10).place("b", 20).started("b", "a", 15).under("a", "b", 30);
        let i = w.index();
        assert_eq!(i.parent("a"), None);
        assert_eq!(i.parent("b").as_deref(), Some("a"));
    }

    #[test]
    fn under_wins_the_drawing_and_never_the_report() {
        let w = family().place("x", 5).under("c", "x", 40);
        let i = w.index();
        assert_eq!(i.parent("c").as_deref(), Some("x"));
        assert_eq!(i.route("c", &|s| s == "b" || s == "x").to, "b", "reports still follow the starter");
        let l = i.lineage("c", &nobody);
        assert_eq!(l.under.map(|u| u.place), Some("x".into()));
        assert_eq!(l.starter.map(|s| s.place), Some("b".into()), "started_by survives adoption");
        // Adopted to the top level.
        let w = family().under("c", MAIN, 40);
        assert_eq!(w.index().parent("c"), None);
    }

    #[test]
    fn handed_to_outranks_started_by_for_drawing_and_reports() {
        // b was removed; c was handed to a.
        let w = World::new().place("a", 10).place("c", 30).started("c", "b", 25).handed("c", "a", 50, "b");
        let i = w.index();
        assert_eq!(i.parent("c").as_deref(), Some("a"));
        assert_eq!(i.route("c", &|s| s == "a").to, "a");
        let l = i.lineage("c", &nobody);
        assert!(l.starter.unwrap().removed);
        assert_eq!(l.handed_to.and_then(|h| h.from), Some("b".into()));
        assert_eq!(i.lineage("a", &nobody).lanes, vec!["c".to_string()]);
        assert!(i.lineage("a", &nobody).started.is_empty());
        assert_eq!(i.lineage("c", &nobody).up.as_deref(), Some("a"));
        assert_eq!(i.lineage("a", &nobody).up, None);
    }

    #[test]
    fn a_report_goes_to_the_nearest_live_ancestor() {
        let w = family();
        let i = w.index();
        assert_eq!(i.route("c", &|s| s == "b" || s == "a"), Route { to: "b".into(), skipped: vec![] });
        // b closed: skipped, a gets it.
        assert_eq!(i.route("c", &|s| s == "a"), Route { to: "a".into(), skipped: vec!["b".into()] });
        // nothing live: (main), live or not.
        assert_eq!(i.route("c", &nobody), Route { to: MAIN.into(), skipped: vec!["b".into(), "a".into()] });
        // b reopened: back to b, nothing written.
        assert_eq!(i.route("c", &|s| s == "b").to, "b");
    }

    #[test]
    fn a_starter_in_another_project_is_shown_and_skipped() {
        let mut w = World::new().place("c", 30);
        let mut l = Link::to("design", 25);
        l.project = Some("other".into());
        w.decl.entry("c".into()).or_default().started_by = Some(l);
        let i = w.index();
        assert_eq!(i.parent("c"), None, "never nests across projects");
        assert_eq!(i.route("c", &|_| true).to, MAIN);
        let s = i.lineage("c", &nobody).starter.unwrap();
        assert_eq!((s.project.as_deref(), s.place.as_str(), s.removed), (Some("other"), "design", false));
    }

    #[test]
    fn resolve_all_asks_liveness_once_per_place_and_only_on_chains() {
        let w = family().place("lone", 5);
        let asked = RefCell::new(Vec::<String>::new());
        let live = |s: &str| {
            asked.borrow_mut().push(s.to_string());
            false
        };
        let all = w.index().resolve_all(&live);
        assert_eq!(all["c"].reports_to.as_deref(), Some(MAIN));
        let mut a = asked.into_inner();
        a.sort();
        assert_eq!(a, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn handoff_names_the_lanes_and_the_next_place_up() {
        let w = family().place("d", 35).started("d", "b", 33).place("e", 36).started("e", "x", 34).under("e", "b", 37);
        let i = w.index();
        // Removing b: c and d move to a; e was only DRAWN under b and moves nowhere.
        assert_eq!(i.handoff("b"), (vec!["c".to_string(), "d".to_string()], "a".to_string()));
        // Removing a top-level starter hands to (main).
        assert_eq!(i.handoff("a"), (vec!["b".to_string()], MAIN.to_string()));
        assert_eq!(i.handoff("c"), (vec![], "b".to_string()));
    }

    #[test]
    fn check_under_refuses_main_unknown_self_and_cycles() {
        let w = family();
        let i = w.index();
        assert!(i.check_under("c", MAIN).is_ok());
        assert!(i.check_under("c", "a").is_ok());
        assert!(i.check_under(MAIN, "a").is_err());
        assert!(i.check_under("nope", "a").is_err());
        assert!(i.check_under("c", "nope").is_err());
        assert!(i.check_under("c", "c").is_err());
        let e = i.check_under("a", "c").unwrap_err();
        assert!(e.contains("cycle") || e.contains("under itself"), "{e}");
    }
}

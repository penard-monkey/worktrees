//! The delivery rules (the design's "Delivery rules (host)"), pure: given the
//! waiting set and a clock, what to send next. Nothing here reads a file, the
//! time or the network, so the 30 s and the 15 minutes run on scripted stamps.
//!
//! **A wait** is a place's unbroken stay in `sessions:busy`'s waiting set: it
//! begins when the place enters and ends when it leaves. Those sets are what
//! the amber dot draws, so a prompt answered and followed by another inside one
//! 3 s poll reads as one wait; its reminder is still "this place has waited 15
//! minutes". In memory only, so a serve restart begins every wait again, and a
//! prompt still up notifies once more 30 s later.

use std::collections::{HashMap, VecDeque};

use worktrees_core::authz::DeviceId;

use super::payload::{Alert, Kind, Place};

/// Answered within this at the Mac, a prompt never reaches the phone.
pub const WAIT_SECS: i64 = 30;
/// Once per wait, this long after it began.
pub const REMINDER_SECS: i64 = 15 * 60;
/// At most one needs-input push per device per this.
pub const BATCH_SECS: i64 = 30;
/// A device's pushes an hour; past that, only the badge moves.
pub const HOURLY: usize = 20;
const HOUR: i64 = 3600;
/// At most one badge-only push per device per this.
pub const BADGE_SECS: i64 = 60;
/// At most one security push per device per this: a flood of refused knocks
/// must not spend the hour's 20 and silence needs-input.
pub const SECURITY_SECS: i64 = 60;
/// Alerts held for one device. Past it, the oldest is dropped and counted.
const SECURITY_QUEUE: usize = 8;
/// A failed send's first pause, doubled to [`MAX_PAUSE`].
const FIRST_PAUSE: i64 = 30;
const MAX_PAUSE: i64 = 15 * 60;

/// How a send ended. The words are constants, never Apple's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    /// 410, or 400 `BadDeviceToken` / `DeviceTokenNotForTopic`.
    Dead(&'static str),
    /// Worth trying again: 429, 5xx, no connection, a timeout, a trust failure.
    Retry(&'static str),
    /// Our provider token: everything waits.
    Auth(&'static str),
    /// Apple refused this one; sending it again would not help.
    Rejected(&'static str),
}

/// One device's switches and what it may hear now: projected by `scope`, muted
/// places already removed.
pub struct View<'a> {
    pub input: bool,
    pub reminder: bool,
    pub security: bool,
    pub waiting: &'a [Place],
}

/// What this push marks as told, undone if it must be sent again.
#[derive(Debug)]
pub struct Planned {
    pub kind: Kind,
    pub badge: u32,
    told: Vec<(String, i64)>,
    reminded: Vec<(String, i64)>,
    counted: bool,
    was_input: bool,
    was_alert: Option<Alert>,
    at: i64,
}

/// The audit's counts since the last record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub sent: u64,
    pub badge: u64,
    pub failed: u64,
    /// Past the hour's 20, or an alert the queue had no room for.
    pub suppressed: u64,
}

impl Tally {
    pub fn any(&self) -> bool {
        *self != Tally::default()
    }
}

#[derive(Default)]
struct Dev {
    /// When each counted push went, inside the last hour.
    sent: VecDeque<i64>,
    last_input: Option<i64>,
    last_badge: Option<i64>,
    last_security: Option<i64>,
    /// The badge the phone last heard from a push that arrived.
    told_badge: Option<u32>,
    /// Path → the wait's start this device has been told about.
    told: HashMap<String, i64>,
    reminded: HashMap<String, i64>,
    pause_until: i64,
    next_pause: i64,
    busy: bool,
    alerts: VecDeque<Alert>,
    tally: Tally,
}

impl Dev {
    fn room(&mut self, now: i64) -> bool {
        while self.sent.front().is_some_and(|t| now - *t >= HOUR) {
            self.sent.pop_front();
        }
        self.sent.len() < HOURLY
    }
}

#[derive(Default)]
pub struct Planner {
    /// Path → when its wait began.
    waits: HashMap<String, i64>,
    devices: HashMap<DeviceId, Dev>,
    /// A provider-token failure stops every device.
    global_pause: i64,
    next_global: i64,
}

impl Planner {
    /// The waiting set as it stands now.
    pub fn observe(&mut self, now: i64, waiting: &[String]) {
        self.waits.retain(|p, _| waiting.contains(p));
        for p in waiting {
            self.waits.entry(p.clone()).or_insert(now);
        }
    }

    /// Hold `alert` for this device: at most [`SECURITY_QUEUE`].
    pub fn queue_alert(&mut self, id: &DeviceId, alert: Alert) {
        let d = self.devices.entry(id.clone()).or_default();
        if d.alerts.len() >= SECURITY_QUEUE {
            d.alerts.pop_front();
            d.tally.suppressed += 1;
        }
        d.alerts.push_back(alert);
    }

    pub fn forget(&mut self, id: &DeviceId) {
        self.devices.remove(id);
    }

    /// `push_test` counts against the hour too. `false` when it has no room.
    pub fn take_test(&mut self, id: &DeviceId, now: i64) -> bool {
        let d = self.devices.entry(id.clone()).or_default();
        if !d.room(now) {
            return false;
        }
        d.sent.push_back(now);
        true
    }

    /// The next push for this device, if one is due. It is `busy` until
    /// [`Planner::report`], so one device has one push in the air at a time.
    pub fn plan(&mut self, now: i64, id: &DeviceId, v: &View) -> Option<Planned> {
        let waits = &self.waits;
        let d = self.devices.entry(id.clone()).or_default();
        d.told.retain(|p, b| waits.get(p) == Some(b));
        d.reminded.retain(|p, b| waits.get(p) == Some(b));
        if d.busy || now < d.pause_until || now < self.global_pause {
            return None;
        }
        // Only what has waited its 30 s counts: a prompt answered at the Mac
        // inside them must not reach the phone, not even as a badge.
        let due: Vec<(&Place, i64)> = v
            .waiting
            .iter()
            .filter_map(|p| waits.get(&p.path).map(|b| (p, *b)))
            .filter(|(_, b)| now - *b >= WAIT_SECS)
            .collect();
        let badge = if v.input { due.len() as u32 } else { 0 };
        let plan = |kind, told, reminded, counted, was_input, was_alert| Planned { kind, badge, told, reminded, counted, was_input, was_alert, at: now };

        if !v.security {
            d.alerts.clear();
        }
        if !d.alerts.is_empty() && d.last_security.is_none_or(|t| now - t >= SECURITY_SECS) {
            let a = d.alerts.pop_front().expect("not empty");
            if d.room(now) {
                d.sent.push_back(now);
                d.last_security = Some(now);
                d.busy = true;
                return Some(plan(Kind::Security(a.clone()), Vec::new(), Vec::new(), true, false, Some(a)));
            }
            d.tally.suppressed += 1;
        }

        let paced = d.last_input.is_none_or(|t| now - t >= BATCH_SECS);
        if v.input && paced {
            let fresh: Vec<(&Place, i64)> = due.iter().copied().filter(|(p, b)| d.told.get(&p.path) != Some(b)).collect();
            if !fresh.is_empty() {
                let told: Vec<(String, i64)> = fresh.iter().map(|(p, b)| (p.path.clone(), *b)).collect();
                // First told at or past 15 minutes: that was the reminder.
                let reminded: Vec<(String, i64)> = fresh.iter().filter(|(_, b)| now - *b >= REMINDER_SECS).map(|(p, b)| (p.path.clone(), *b)).collect();
                let oldest = fresh.iter().map(|(_, b)| *b).min().unwrap_or(now);
                d.told.extend(told.iter().cloned());
                d.reminded.extend(reminded.iter().cloned());
                if d.room(now) {
                    d.sent.push_back(now);
                    d.last_input = Some(now);
                    d.busy = true;
                    let kind = match fresh.as_slice() {
                        [(p, b)] => Kind::Input { place: (*p).clone(), waited_s: now - *b },
                        _ => Kind::Several { places: fresh.iter().map(|(p, _)| (*p).clone()).collect(), count: fresh.len(), waited_s: now - oldest },
                    };
                    return Some(plan(kind, told, reminded, true, true, None));
                }
                d.tally.suppressed += fresh.len() as u64;
            }
            if v.reminder {
                let next = due.iter().copied().find(|(p, b)| now - *b >= REMINDER_SECS && d.told.get(&p.path) == Some(b) && d.reminded.get(&p.path) != Some(b));
                if let Some((p, b)) = next {
                    let r = vec![(p.path.clone(), b)];
                    d.reminded.insert(p.path.clone(), b);
                    if d.room(now) {
                        d.sent.push_back(now);
                        d.last_input = Some(now);
                        d.busy = true;
                        return Some(plan(Kind::Reminder { place: p.clone(), waited_s: now - b }, Vec::new(), r, true, true, None));
                    }
                    d.tally.suppressed += 1;
                }
            }
        }

        if badge != d.told_badge.unwrap_or(0) && d.last_badge.is_none_or(|t| now - t >= BADGE_SECS) {
            d.last_badge = Some(now);
            d.busy = true;
            return Some(plan(Kind::Badge, Vec::new(), Vec::new(), false, false, None));
        }
        None
    }

    /// How `p`, for device `id`, ended, at `now`.
    pub fn report(&mut self, id: &DeviceId, p: &Planned, how: &Delivery, now: i64) {
        let is_badge = matches!(p.kind, Kind::Badge);
        if let Delivery::Auth(_) = how {
            let step = self.next_global.clamp(FIRST_PAUSE * 10, MAX_PAUSE);
            self.global_pause = now + step;
            self.next_global = (step * 2).min(MAX_PAUSE);
        } else if matches!(how, Delivery::Sent) {
            self.next_global = 0;
        }
        let Some(d) = self.devices.get_mut(id) else { return };
        d.busy = false;
        match how {
            Delivery::Sent => {
                d.told_badge = Some(p.badge);
                d.next_pause = 0;
                if is_badge {
                    d.tally.badge += 1;
                } else {
                    d.tally.sent += 1;
                }
            }
            // Not to be tried again, and that goes for the badge it carried: a
            // refusal must not become a badge push a minute, for ever.
            Delivery::Dead(_) | Delivery::Rejected(_) => {
                d.told_badge = Some(p.badge);
                d.tally.failed += 1;
            }
            Delivery::Retry(_) | Delivery::Auth(_) => {
                d.tally.failed += 1;
                // Not told after all: what is still due folds into the next.
                for (path, _) in &p.told {
                    d.told.remove(path);
                }
                for (path, _) in &p.reminded {
                    d.reminded.remove(path);
                }
                if p.counted {
                    if let Some(i) = d.sent.iter().rposition(|t| *t == p.at) {
                        d.sent.remove(i);
                    }
                }
                if p.was_input {
                    d.last_input = None;
                }
                if let Some(a) = &p.was_alert {
                    d.alerts.push_front(a.clone());
                    d.last_security = None;
                }
                if is_badge {
                    d.last_badge = None;
                }
                if matches!(how, Delivery::Retry(_)) {
                    let step = d.next_pause.clamp(FIRST_PAUSE, MAX_PAUSE);
                    d.pause_until = now + step;
                    d.next_pause = (step * 2).min(MAX_PAUSE);
                }
            }
        }
    }

    /// A send that will never report: aborted by the kill switch.
    pub fn abandon(&mut self, id: &DeviceId, p: &Planned) {
        if let Some(d) = self.devices.get_mut(id) {
            d.busy = false;
            for (path, _) in &p.told {
                d.told.remove(path);
            }
            for (path, _) in &p.reminded {
                d.reminded.remove(path);
            }
            if p.counted {
                if let Some(i) = d.sent.iter().rposition(|t| *t == p.at) {
                    d.sent.remove(i);
                }
            }
            if p.was_input {
                d.last_input = None;
            }
        }
    }

    /// Everything counted since the last call, per device.
    pub fn take_tallies(&mut self) -> Vec<(DeviceId, Tally)> {
        let mut out: Vec<(DeviceId, Tally)> = self
            .devices
            .iter_mut()
            .filter(|(_, d)| d.tally.any())
            .map(|(id, d)| (id.clone(), std::mem::take(&mut d.tally)))
            .collect();
        out.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(n: &str) -> Place {
        Place { path: format!("/r/{n}"), repo: "/r".into(), slug: n.into(), project: "r".into(), place: n.into(), agent: Some("Claude".into()) }
    }

    fn id() -> DeviceId {
        DeviceId::new("d_phone")
    }

    struct Sim {
        p: Planner,
        places: Vec<Place>,
        input: bool,
        reminder: bool,
        security: bool,
        sent: Vec<(i64, Kind, u32)>,
        how: Delivery,
    }

    impl Sim {
        fn new() -> Sim {
            Sim { p: Planner::default(), places: Vec::new(), input: true, reminder: true, security: false, sent: Vec::new(), how: Delivery::Sent }
        }
        fn set(&mut self, now: i64, names: &[&str]) {
            self.places = names.iter().map(|n| place(n)).collect();
            let paths: Vec<String> = self.places.iter().map(|p| p.path.clone()).collect();
            self.p.observe(now, &paths);
        }
        /// One tick: at most one push, reported at once.
        fn tick(&mut self, now: i64) -> Option<Kind> {
            let paths: Vec<String> = self.places.iter().map(|p| p.path.clone()).collect();
            self.p.observe(now, &paths);
            let v = View { input: self.input, reminder: self.reminder, security: self.security, waiting: &self.places };
            let planned = self.p.plan(now, &id(), &v)?;
            self.p.report(&id(), &planned, &self.how, now);
            self.sent.push((now, planned.kind.clone(), planned.badge));
            Some(planned.kind)
        }
        fn run(&mut self, from: i64, to: i64) {
            for t in from..=to {
                self.tick(t);
            }
        }
        fn words(&self) -> Vec<(i64, &'static str, u32)> {
            self.sent.iter().map(|(t, k, b)| (*t, k.word(), *b)).collect()
        }
    }

    /// Answered inside the 30 s, a prompt never reaches the phone: no push, no
    /// badge. At exactly 30 s it goes, once, and the badge says so.
    #[test]
    fn thirty_seconds_at_the_mac_never_reach_the_phone() {
        let mut s = Sim::new();
        s.set(0, &["a"]);
        s.run(0, 29);
        assert_eq!(s.words(), vec![], "nothing inside the wait");
        s.set(29, &[]);
        s.run(29, 200);
        assert_eq!(s.words(), vec![], "answered at the Mac: not even a badge");
        s.set(300, &["a"]);
        s.run(300, 340);
        assert_eq!(s.words(), vec![(330, "input", 1)], "a wait's push, exactly once");
        let Kind::Input { waited_s, .. } = &s.sent[0].1 else { panic!() };
        assert_eq!(*waited_s, 30);
    }

    /// One reminder per wait, at 15 minutes, never again however long it
    /// waits; a new wait earns its own.
    #[test]
    fn one_reminder_per_prompt() {
        let mut s = Sim::new();
        s.set(0, &["a"]);
        s.run(0, 5000);
        assert_eq!(s.words(), vec![(30, "input", 1), (900, "reminder", 1)]);
        s.set(5001, &[]);
        s.run(5001, 5100);
        s.set(5101, &["a"]);
        s.run(5101, 6100);
        assert_eq!(s.words()[3..], [(5131, "input", 1), (6001, "reminder", 1)], "{:?}", s.words());
    }

    #[test]
    fn the_reminder_switch_stops_only_the_reminder() {
        let mut s = Sim::new();
        s.reminder = false;
        s.set(0, &["a"]);
        s.run(0, 2000);
        assert_eq!(s.words(), vec![(30, "input", 1)]);
    }

    /// A phone that registers 20 minutes into a wait hears one "needs input",
    /// not that and a "still waiting" 30 s later.
    #[test]
    fn a_first_push_past_fifteen_minutes_is_also_the_reminder() {
        let mut s = Sim::new();
        s.input = false;
        s.set(0, &["a"]);
        s.run(0, 1200);
        assert_eq!(s.words(), vec![]);
        s.input = true;
        s.run(1201, 3000);
        assert_eq!(s.words(), vec![(1201, "input", 1)]);
    }

    /// Windows that end together fold into one `several`; a place due while
    /// the last push is under 30 s old waits for the next, and the two then
    /// fold. At most one per 30 s, whatever comes due.
    #[test]
    fn places_due_together_fold_and_the_pace_is_thirty_seconds() {
        let mut s = Sim::new();
        s.set(0, &["a", "b", "c"]);
        s.run(0, 40);
        assert_eq!(s.words(), vec![(30, "several", 3)]);
        let Kind::Several { count, places, .. } = &s.sent[0].1 else { panic!() };
        assert_eq!((*count, places.len()), (3, 3));
        // b's and c's windows end at 35 and 45, under 30 s from the last push.
        let mut s = Sim::new();
        s.set(0, &["a"]);
        s.run(0, 4);
        s.set(5, &["a", "b"]);
        s.run(5, 14);
        s.set(15, &["a", "b", "c"]);
        s.run(15, 70);
        let alerts: Vec<_> = s.words().into_iter().filter(|w| w.1 != "badge").collect();
        assert_eq!(alerts, vec![(30, "input", 1), (60, "several", 3)], "b and c fold at the next slot");
    }

    /// 20 an hour, flooded past: then only the badge moves, and a place that
    /// came due in that time does not fire late. An hour on there is room again.
    #[test]
    fn twenty_an_hour_then_only_the_badge() {
        let mut s = Sim::new();
        let mut now = 0;
        // A place that comes and goes, 60 s up and 40 s down, far past 20.
        for _ in 0..30 {
            s.set(now, &["a"]);
            s.run(now, now + 60);
            s.set(now + 61, &[]);
            s.run(now + 61, now + 100);
            now += 101;
        }
        let alerts = s.words().iter().filter(|(_, w, _)| *w != "badge").count();
        assert_eq!(alerts, HOURLY, "the flood reached the cap and stopped: {:?}", s.words());
        assert!(now < HOUR, "the flood ran inside one hour: {now}");
        assert!(s.words().last().unwrap().1 == "badge", "the badge still moves: {:?}", s.words().last());
        assert_eq!(s.p.take_tallies()[0].1.suppressed, 10, "30 waits, 20 pushes");
        let later = HOUR + 100;
        s.set(later, &["a"]);
        s.run(later, later + 40);
        assert_eq!(s.words().last().unwrap().1, "input", "an hour on");
    }

    /// The badge counts places past their 30 s, follows the count down to 0,
    /// at most once a minute; an input switched off counts nothing.
    #[test]
    fn the_badge_follows_the_count_at_most_once_a_minute() {
        let mut s = Sim::new();
        s.set(0, &["a", "b"]);
        s.run(0, 31);
        assert_eq!(s.words(), vec![(30, "several", 2)]);
        s.set(32, &["a"]);
        s.run(32, 34);
        assert_eq!(s.words().last().unwrap(), &(32, "badge", 1), "a badge-only push");
        assert_eq!(s.sent.last().unwrap().1.priority(), 5);
        s.set(35, &[]);
        s.run(35, 91);
        assert_eq!(s.words().last().unwrap().0, 32, "not again inside the minute");
        s.run(92, 93);
        assert_eq!(s.words().last().unwrap(), &(92, "badge", 0), "down to 0, a minute on");
    }

    #[test]
    fn with_input_off_nothing_about_waits_is_sent() {
        let mut s = Sim::new();
        s.input = false;
        s.set(0, &["a"]);
        s.run(0, 3000);
        assert_eq!(s.words(), vec![]);
    }

    /// A failed send is not told: what is due folds into the next try, after a
    /// pause that doubles to its ceiling, and a success ends that.
    #[test]
    fn a_retry_backs_off_and_folds() {
        let mut s = Sim::new();
        s.how = Delivery::Retry("status-503");
        s.set(0, &["a"]);
        s.run(0, 30);
        assert_eq!(s.words(), vec![(30, "input", 1)]);
        s.run(31, 59);
        assert_eq!(s.sent.len(), 1, "paused 30 s");
        s.run(60, 60);
        assert_eq!(s.sent.len(), 2);
        s.run(61, 89);
        s.set(90, &["a", "b"]);
        s.run(90, 119);
        assert_eq!(s.sent.len(), 2, "paused 60 s");
        s.how = Delivery::Sent;
        s.run(120, 121);
        assert_eq!(s.words().last().unwrap(), &(120, "several", 2), "b came due meanwhile, and folded: {:?}", s.words());
        let hour_used = s.p.devices[&id()].sent.len();
        assert_eq!(hour_used, 1, "only what arrived counts against the hour");
        assert_eq!(s.p.devices[&id()].next_pause, 0, "a success ends the backoff");
    }

    /// A dead token is told: nothing to retry, and no storm of retries.
    #[test]
    fn dead_and_rejected_are_not_tried_again() {
        for how in [Delivery::Dead("unregistered"), Delivery::Rejected("payload-too-large")] {
            let mut s = Sim::new();
            s.how = how;
            s.set(0, &["a"]);
            s.run(0, 500);
            assert_eq!(s.words(), vec![(30, "input", 1)]);
        }
    }

    /// A bad provider token stops every device: Apple punishes repeats.
    #[test]
    fn an_auth_failure_stops_every_device() {
        let mut p = Planner::default();
        p.observe(0, &["/r/a".to_string()]);
        let places = [place("a")];
        let v = View { input: true, reminder: true, security: false, waiting: &places };
        let (a, b) = (DeviceId::new("a"), DeviceId::new("b"));
        let x = p.plan(30, &a, &v).unwrap();
        p.report(&a, &x, &Delivery::Auth("invalid-provider-token"), 30);
        assert!(p.plan(31, &b, &v).is_none(), "another device waits too");
        assert!(p.plan(30 + 300, &b, &v).is_some(), "after five minutes");
    }

    /// One device, one push in the air: nothing piles up behind a slow Apple.
    #[test]
    fn a_device_has_one_push_in_the_air() {
        let mut p = Planner::default();
        p.observe(0, &["/r/a".to_string()]);
        let places = [place("a")];
        let v = View { input: true, reminder: true, security: false, waiting: &places };
        let x = p.plan(30, &id(), &v).unwrap();
        assert!(p.plan(5000, &id(), &v).is_none(), "still in the air");
        p.abandon(&id(), &x);
        assert!(p.plan(5001, &id(), &v).is_some(), "abandoned, still due");
    }

    /// Security alerts wait a minute between them, are held in a short queue,
    /// and a switched-off device holds none.
    #[test]
    fn security_alerts_are_paced_and_bounded() {
        let mut s = Sim::new();
        s.security = true;
        for _ in 0..12 {
            s.p.queue_alert(&id(), Alert { what: "refused", device: None, reason: None });
        }
        s.run(0, 200);
        assert_eq!(s.words().iter().map(|w| w.0).collect::<Vec<_>>(), vec![0, 60, 120, 180]);
        s.security = false;
        s.run(240, 300);
        assert_eq!(s.sent.len(), 4, "off: the rest are gone");
        s.security = true;
        s.run(301, 400);
        assert_eq!(s.sent.len(), 4);
        assert_eq!(s.p.take_tallies()[0].1.suppressed, 4, "12 into a queue of 8");
    }

    /// `push_test` spends the same hour.
    #[test]
    fn a_test_counts_against_the_hour() {
        let mut p = Planner::default();
        assert_eq!((0..HOURLY + 5).filter(|_| p.take_test(&id(), 100)).count(), HOURLY);
        assert!(p.take_test(&id(), 100 + HOUR));
    }
}

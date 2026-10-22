//! Single-flight for the poll-shaped reads (Phase 2 §6).
//!
//! Coalescing beats rate-limiting for them: every client polls the same few
//! reads, and each is a fan-out (git across every place, the GitHub API, the
//! usage files). Measured against 72e7339: one `list_workspace` over one
//! project was 11 git processes, and ten clients asking at once were 110.
//! Concurrent IDENTICAL calls now share one run of the BODY: the first runs
//! it, the rest wait for its outcome, and each gets the answer under its own
//! id. A call that arrives after the answer is out starts a new one, so
//! nothing here is a cache.
//!
//! **It sits behind the resolver and in front of the lane gate**
//! (`dispatch::call_in_seen`): every call is resolved for its own caller
//! first, so a refused one never joins a flight, and what is shared is the
//! body's outcome alone, which each caller's row filter then cuts down to its
//! own grant (§3.3). Only a flight's leader meets a device launch's lane gate
//! (`provenance::device_lane_gate`), whose lock is held across the body: a
//! follower that met it first would queue behind its own leader and then run
//! the body again. The body runs on a `Ctx`
//! scoped to the caller's grant and fetching as that caller, so the key
//! carries both: two calls share only when their bodies would see the same
//! projects and fetch the same way. The console and a device scoped `"*"`
//! share; devices with different scopes do not.
//!
//! A follower waits on the thread of the job its request runs in, holding its
//! in-flight slot, as a body would. It saves the work — git, the GitHub API,
//! the disk — not the thread; the in-flight caps bound the threads.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use serde_json::Value;
use worktrees_core::authz::Projects;
use worktrees_core::fetchpolicy::Fetcher;

use super::dispatch::Args;
use super::wire::INTERNAL_ERROR;

/// The rows that share: §6's list.
pub const SHARED: &[&str] = &[
    "list_workspace",
    "project_prs",
    "claude_usage",
    "codex_usage",
    "kimi_usage",
    "doctor",
    "diagnostics",
    "list_releases",
    "check_update",
];

/// A body's outcome, as the wire's `ok` / `err`.
pub type Outcome = Result<Value, Value>;

/// `value` with every object's keys sorted, as text: `args` keeps the order
/// the client wrote (serde_json's `preserve_order`), and two clients that
/// spell the same arguments in two orders are asking the same thing.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys.iter().map(|k| format!("{}:{}", Value::from(k.as_str()), canonical(&m[*k]))).collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(",")),
        v => v.to_string(),
    }
}

/// The flight a call joins, or `None` for a row that never shares: the
/// command, its arguments as the resolver left them (canonical objects), and
/// what its body sees — the scope and the fetcher of the `Ctx` it runs on.
pub fn key(cmd: &str, args: &Args, scope: &Projects, fetcher: &Fetcher) -> Option<String> {
    #[cfg(test)]
    let shares = SHARED.contains(&cmd) || cmd == super::dispatch::TEST_SHARED;
    #[cfg(not(test))]
    let shares = SHARED.contains(&cmd);
    shares.then(|| {
        let scope = serde_json::to_string(scope).unwrap_or_default();
        format!("{cmd} {} scope={scope} fetch={fetcher:?}", canonical(&Value::Object(args.0.clone())))
    })
}

/// One flight: the leader's outcome, once it lands.
#[derive(Default)]
pub struct Flight {
    answer: Mutex<Option<Arc<Outcome>>>,
    landed: Condvar,
}

impl Flight {
    /// Publish `outcome` unless one is out already, and wake every follower.
    fn publish(&self, outcome: Outcome) {
        let mut a = self.answer.lock().unwrap();
        if a.is_none() {
            *a = Some(Arc::new(outcome));
            self.landed.notify_all();
        }
    }

    /// The leader's outcome, once it lands.
    pub fn wait(&self) -> Outcome {
        let mut a = self.answer.lock().unwrap();
        while a.is_none() {
            a = self.landed.wait(a).unwrap();
        }
        (**a.as_ref().expect("waited for Some")).clone()
    }
}

/// The flights in the air, host-wide: every connection's identical call
/// joins the same one.
#[derive(Default)]
pub struct Flights {
    map: Mutex<HashMap<String, Arc<Flight>>>,
    /// Calls that ever joined a flight, leading or following.
    #[cfg(test)]
    joins: std::sync::atomic::AtomicUsize,
}

pub enum Join {
    /// Run the body, then `Lead::land` with what it answered.
    Lead(Lead),
    /// Wait for the leader's outcome.
    Follow(Arc<Flight>),
}

/// The caller that runs the body. Landing — or being dropped without
/// landing, as a leader whose body panics is, on its way out — takes the
/// flight out of the map, so the next identical call starts a new one.
pub struct Lead {
    key: String,
    flights: Arc<Flights>,
    flight: Arc<Flight>,
}

impl Flights {
    /// `body`'s outcome, run once for every concurrent call with `key`: the
    /// first runs it, the rest wait and get the same. A leader whose body
    /// panics answers its followers `internal error`, and the panic goes on
    /// to the leader's own `catch_unwind` (`conn::answer`). Test builds only:
    /// `dispatch::call_in_seen` writes this out with [`Flights::join`],
    /// because a follower is witnessed there and a leader meets the lane
    /// gate first.
    #[cfg(test)]
    pub fn share(self: &Arc<Self>, key: String, body: impl FnOnce() -> Outcome) -> Outcome {
        match self.join(key) {
            Join::Follow(f) => f.wait(),
            Join::Lead(lead) => {
                let out = body();
                lead.land(&out);
                out
            }
        }
    }

    /// Join the flight in the air for `key`, or lead a new one.
    pub fn join(self: &Arc<Self>, key: String) -> Join {
        #[cfg(test)]
        self.joins.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut map = self.map.lock().unwrap();
        if let Some(f) = map.get(&key) {
            return Join::Follow(f.clone());
        }
        let flight = Arc::new(Flight::default());
        map.insert(key.clone(), flight.clone());
        Join::Lead(Lead { key, flights: self.clone(), flight })
    }

    /// Flights in the air right now.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }

    /// Calls that have joined a flight so far.
    #[cfg(test)]
    pub fn joins(&self) -> usize {
        self.joins.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Lead {
    /// Out of the map, if this flight is still the one there.
    fn leave(&self) {
        let mut map = self.flights.map.lock().unwrap();
        if map.get(&self.key).is_some_and(|f| Arc::ptr_eq(f, &self.flight)) {
            map.remove(&self.key);
        }
    }

    /// Publish the outcome to every follower. Out of the map FIRST: a caller
    /// that joins after this starts its own flight, and every follower that
    /// joined before it holds the flight that gets the outcome.
    pub fn land(self, outcome: &Outcome) {
        self.leave();
        self.flight.publish(outcome.clone());
    }
}

impl Drop for Lead {
    fn drop(&mut self) {
        // Landed: already gone and published, and this does nothing. Dropped
        // unlanded: the followers get `internal error` rather than a wait
        // that never ends.
        self.leave();
        self.flight.publish(Err(Value::from(INTERNAL_ERROR)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(v: Value) -> Args {
        Args(v.as_object().cloned().unwrap_or_default())
    }

    fn k(cmd: &str, v: Value) -> Option<String> {
        key(cmd, &args(v), &Projects::All, &Fetcher::Console)
    }

    /// A follower's wait, on its own thread and bounded: a follower that is
    /// never answered fails the test rather than hanging it.
    fn waiting(fl: Arc<Flight>) -> std::sync::mpsc::Receiver<Outcome> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(fl.wait());
        });
        rx
    }

    fn answered(rx: &std::sync::mpsc::Receiver<Outcome>) -> Outcome {
        rx.recv_timeout(std::time::Duration::from_secs(5)).expect("the follower was never answered")
    }

    #[test]
    fn identical_calls_share_a_key_whatever_order_their_keys_came_in() {
        let a = k("doctor", json!({ "repo": "/r", "slug": "s" }));
        let b = k("doctor", json!({ "slug": "s", "repo": "/r" }));
        assert!(a.is_some());
        assert_eq!(a, b);
        assert_ne!(a, k("doctor", json!({ "repo": "/r", "slug": "t" })));
        assert_ne!(k("list_workspace", json!({})), k("diagnostics", json!({})));
        assert_eq!(k("set_note", json!({})), None, "a mutation never shares");
        assert_eq!(k("read_file", json!({ "path": "/x" })), None, "nor a read §6 did not name");
        let nested = |v: Value| k("project_prs", json!({ "repo": "/r", "places": [v] }));
        assert_eq!(nested(json!({ "a": 1, "b": 2 })), nested(json!({ "b": 2, "a": 1 })));
    }

    /// What the body sees is part of the key: a scoped device's
    /// `list_workspace` is not the console's, and neither is a device's
    /// fetching. The console and a device scoped `"*"` with nothing to fetch
    /// differ only in the fetcher.
    #[test]
    fn the_key_carries_the_bodys_scope_and_fetcher() {
        let a = args(json!({}));
        let all = key("list_workspace", &a, &Projects::All, &Fetcher::Console);
        let one = key("list_workspace", &a, &Projects::only(["/r/a"]), &Fetcher::Console);
        let two = key("list_workspace", &a, &Projects::only(["/r/b", "/r/a"]), &Fetcher::Console);
        assert_ne!(all, one, "a scoped body sees fewer projects");
        assert_ne!(one, two);
        assert_eq!(two, key("list_workspace", &a, &Projects::only(["/r/a", "/r/b"]), &Fetcher::Console), "a scope is a set");
        assert_ne!(all, key("list_workspace", &a, &Projects::All, &Fetcher::Device(vec![])), "a device fetches by its own rule");
    }

    #[test]
    fn every_shared_row_is_a_row_of_the_table() {
        for row in SHARED {
            assert!(super::super::dispatch::exposure(row).is_some(), "{row}");
        }
    }

    /// The leader lands, every follower gets it, and the flight is gone: the
    /// next identical call leads again.
    #[test]
    fn followers_get_the_leaders_answer_and_the_next_call_starts_over() {
        let f = Arc::new(Flights::default());
        let Join::Lead(lead) = f.join("k".into()) else { panic!("the first call leads") };
        let followers: Vec<_> = (0..3)
            .map(|_| match f.join("k".into()) {
                Join::Follow(fl) => waiting(fl),
                Join::Lead(_) => panic!("a call while one is in the air follows it"),
            })
            .collect();
        lead.land(&Ok(json!(7)));
        assert_eq!(f.len(), 0);
        for rx in &followers {
            assert_eq!(answered(rx), Ok(json!(7)));
        }
        assert!(matches!(f.join("k".into()), Join::Lead(_)), "landed means gone: the next call leads");
    }

    /// A leader dropped without landing — a body that panicked, or a job
    /// dropped unrun — answers its followers `internal error` rather than
    /// leaving them waiting, and leaves no flight behind.
    #[test]
    fn a_leader_that_never_lands_fails_its_followers_and_clears_the_flight() {
        let f = Arc::new(Flights::default());
        let Join::Lead(lead) = f.join("k".into()) else { panic!() };
        let Join::Follow(fl) = f.join("k".into()) else { panic!() };
        let rx = waiting(fl);
        drop(lead);
        assert_eq!(answered(&rx), Err(Value::from(INTERNAL_ERROR)));
        assert_eq!(f.len(), 0);
    }

    /// `share` under a panicking body: the panic reaches the leader's caller,
    /// and its follower is answered all the same.
    #[test]
    fn a_body_that_panics_answers_the_followers_and_unwinds_the_leader() {
        let f = Arc::new(Flights::default());
        let (started, go) = (Arc::new(std::sync::Barrier::new(2)), Arc::new(std::sync::Barrier::new(2)));
        let (f2, s2, g2) = (f.clone(), started.clone(), go.clone());
        let leader = std::thread::spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                f2.share("k".into(), || {
                    s2.wait();
                    g2.wait();
                    panic!("the body fell over")
                })
            }))
        });
        started.wait();
        let Join::Follow(fl) = f.join("k".into()) else { panic!("the leader is in the air") };
        let rx = waiting(fl);
        go.wait();
        assert_eq!(answered(&rx), Err(Value::from(INTERNAL_ERROR)));
        assert!(leader.join().unwrap().is_err(), "the panic went on to the leader's catch_unwind");
        assert_eq!(f.len(), 0);
    }
}

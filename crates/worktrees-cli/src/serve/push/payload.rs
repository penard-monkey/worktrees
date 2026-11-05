//! What a push says: the plaintext the phone opens, and the clear part Apple
//! carries. `docs/push-contract.md` is the wire, `docs/push-fixtures/` its
//! examples, which the tests here hold against these functions.
//!
//! Prompt text never leaves the host, even sealed. A plaintext carries names
//! and a kind, nothing else.

use serde_json::{json, Map, Value};

use super::seal;

/// APNs refuses a larger payload.
pub const MAX_PAYLOAD: usize = 4096;
/// A longest name kept, in characters.
const MAX_NAME: usize = 64;
/// `several` names at most this many places.
pub const SEVERAL_NAMED: usize = 5;

/// The push's `sound`. None: the design's `aps` has none, so a push is silent
/// however it breaks through. (main) is asking; this is the one line to change.
pub const SOUND: Option<&str> = None;

pub const INPUT_HOURS: i64 = 1;
pub const SECURITY_HOURS: i64 = 24;

/// One waiting place, as a device's push names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub path: String,
    pub repo: String,
    pub slug: String,
    pub project: String,
    pub place: String,
    /// The provider's name, `Claude`; the phone says `Agent` for none.
    pub agent: Option<String>,
}

/// A security alert, as the phone's copy rows take it (`notif-sec-*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    /// `enrolled`, `disabled` or `refused`; any other has the phone's generic
    /// words, and `reason` is what it shows.
    pub what: &'static str,
    pub device: Option<String>,
    /// Shown as the notification's body.
    pub reason: Option<String>,
}

/// What one push is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Input { place: Place, waited_s: i64 },
    Reminder { place: Place, waited_s: i64 },
    Several { places: Vec<Place>, count: usize, waited_s: i64 },
    Security(Alert),
    Test,
    /// Only the badge moves: no alert, nothing sealed.
    Badge,
}

impl Kind {
    /// The word logs and the audit's counts use. Never a name.
    pub fn word(&self) -> &'static str {
        match self {
            Kind::Input { .. } => "input",
            Kind::Reminder { .. } => "reminder",
            Kind::Several { .. } => "several",
            Kind::Security(_) => "security",
            Kind::Test => "test",
            Kind::Badge => "badge",
        }
    }

    fn fallback(&self) -> &'static str {
        match self {
            Kind::Input { .. } | Kind::Several { .. } => "A session needs input",
            Kind::Reminder { .. } => "A session is still waiting for input",
            Kind::Security(_) => "Security alert from your host",
            Kind::Test => "Notifications are working",
            Kind::Badge => "",
        }
    }

    fn time_sensitive(&self) -> bool {
        matches!(self, Kind::Input { .. } | Kind::Reminder { .. } | Kind::Several { .. })
    }

    /// The headers' `apns-expiration`, from `now`: an hour for needs input,
    /// a day for security.
    pub fn expiration(&self, now: i64) -> i64 {
        let hours = if matches!(self, Kind::Security(_)) { SECURITY_HOURS } else { INPUT_HOURS };
        now + hours * 3600
    }

    pub fn priority(&self) -> u8 {
        if matches!(self, Kind::Badge) {
            5
        } else {
            10
        }
    }

    /// The place whose notification this one replaces: `input` and `reminder`.
    pub fn collapses(&self) -> Option<&Place> {
        match self {
            Kind::Input { place, .. } | Kind::Reminder { place, .. } => Some(place),
            _ => None,
        }
    }
}

/// A name cut to [`MAX_NAME`] characters.
fn name(s: &str) -> String {
    s.chars().take(MAX_NAME).collect()
}

/// The JSON the phone opens, or `None` for a badge. `places` are at most
/// [`SEVERAL_NAMED`] of those given, fewer if the payload would not fit.
pub fn plaintext(kind: &Kind, host: &str, named: usize) -> Option<Value> {
    let mut m = Map::new();
    m.insert("v".into(), json!(1));
    m.insert("kind".into(), json!(kind.word()));
    m.insert("host".into(), json!(name(host)));
    let place_fields = |m: &mut Map<String, Value>, p: &Place, waited: i64| {
        m.insert("repo".into(), json!(p.repo));
        m.insert("slug".into(), json!(p.slug));
        m.insert("project".into(), json!(name(&p.project)));
        m.insert("place".into(), json!(name(&p.place)));
        if let Some(a) = &p.agent {
            m.insert("agent".into(), json!(name(a)));
        }
        m.insert("waited_s".into(), json!(waited));
    };
    match kind {
        Kind::Badge => return None,
        Kind::Test => {}
        Kind::Input { place, waited_s } | Kind::Reminder { place, waited_s } => place_fields(&mut m, place, *waited_s),
        Kind::Several { places, count, waited_s } => {
            let list: Vec<Value> =
                places.iter().take(named.min(SEVERAL_NAMED)).map(|p| json!({ "project": name(&p.project), "place": name(&p.place) })).collect();
            let held = list.len();
            m.insert("places".into(), Value::Array(list));
            // Only when more came due than `places` holds.
            if *count > held {
                m.insert("count".into(), json!(count));
            }
            m.insert("waited_s".into(), json!(waited_s));
        }
        Kind::Security(a) => {
            m.insert("what".into(), json!(a.what));
            if let Some(d) = &a.device {
                m.insert("device".into(), json!(name(d)));
            }
            if let Some(r) = &a.reason {
                m.insert("reason".into(), json!(r));
            }
        }
    }
    Some(Value::Object(m))
}

/// The clear part: what Apple sees.
pub fn aps(kind: &Kind, badge: u32) -> Value {
    if matches!(kind, Kind::Badge) {
        return json!({ "badge": badge });
    }
    let mut v = json!({
        "alert": { "title": "Worktrees", "body": kind.fallback() },
        "mutable-content": 1,
        "interruption-level": if kind.time_sensitive() { "time-sensitive" } else { "active" },
        "badge": badge,
    });
    if let Some(s) = SOUND {
        v["sound"] = json!(s);
    }
    v
}

/// The whole body, sealed with `nonce`. Names too long for APNs's 4 KB shed
/// places first (`count` still says how many), and past that the push goes
/// with the fallback line alone: a place whose path alone does not fit still
/// tells the person something needs them.
pub fn body(kind: &Kind, badge: u32, host: &str, key: &[u8; seal::KEY_LEN], key_id: &[u8; seal::KEY_ID_LEN], nonce: [u8; seal::NONCE_LEN]) -> Vec<u8> {
    let aps = aps(kind, badge);
    if matches!(kind, Kind::Badge) {
        return json!({ "aps": aps }).to_string().into_bytes();
    }
    for named in (1..=SEVERAL_NAMED).rev() {
        // `several` needs two names for the phone's words; nothing else reads
        // `named`, so the other kinds take the first pass.
        if matches!(kind, Kind::Several { .. }) && named < 2 {
            break;
        }
        let Some(plain) = plaintext(kind, host, named) else { break };
        let wt = seal::wt(key, key_id, nonce, plain.to_string().as_bytes());
        let out = json!({ "aps": aps, "wt": wt }).to_string().into_bytes();
        if out.len() <= MAX_PAYLOAD {
            return out;
        }
    }
    json!({ "aps": aps }).to_string().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(n: &str) -> Place {
        Place { path: format!("/r/{n}/.worktrees/w"), repo: format!("/r/{n}"), slug: "w".into(), project: n.into(), place: "w".into(), agent: Some("Claude".into()) }
    }

    #[test]
    fn apple_sees_a_kind_word_and_a_badge_and_no_name() {
        let k = Kind::Input { place: place("secret-project"), waited_s: 31 };
        let b = body(&k, 2, "my-mac", &[1; 32], &[2; 8], [3; 12]);
        let v: Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["aps"]["alert"]["body"], "A session needs input");
        assert_eq!(v["aps"]["alert"]["title"], "Worktrees");
        assert_eq!(v["aps"]["interruption-level"], "time-sensitive");
        assert_eq!(v["aps"]["mutable-content"], 1);
        assert_eq!(v["aps"]["badge"], 2);
        let clear = String::from_utf8(b).unwrap();
        for name in ["secret-project", "my-mac", "Claude", "/r/"] {
            assert!(!clear.contains(name), "{name} crossed Apple in the clear");
        }
    }

    #[test]
    fn a_badge_push_is_only_the_badge() {
        let v: Value = serde_json::from_slice(&body(&Kind::Badge, 0, "h", &[1; 32], &[2; 8], [3; 12])).unwrap();
        assert_eq!(v, json!({ "aps": { "badge": 0 } }));
        assert_eq!(Kind::Badge.priority(), 5);
        assert_eq!(Kind::Test.priority(), 10);
    }

    #[test]
    fn expirations_are_an_hour_for_input_and_a_day_for_security() {
        assert_eq!(Kind::Input { place: place("a"), waited_s: 1 }.expiration(100), 3700);
        assert_eq!(Kind::Security(Alert { what: "enrolled", device: None, reason: None }).expiration(100), 86_500);
    }

    /// `count` appears only when `places` holds fewer than came due.
    #[test]
    fn several_names_five_and_counts_the_rest() {
        let places: Vec<Place> = (0..7).map(|i| place(&format!("p{i}"))).collect();
        let v = plaintext(&Kind::Several { places: places.clone(), count: 7, waited_s: 30 }, "h", SEVERAL_NAMED).unwrap();
        assert_eq!(v["places"].as_array().unwrap().len(), 5);
        assert_eq!(v["count"], 7);
        let v = plaintext(&Kind::Several { places: places[..2].to_vec(), count: 2, waited_s: 30 }, "h", SEVERAL_NAMED).unwrap();
        assert_eq!(v.get("count"), None);
    }

    /// Long names still fit in APNs's 4 KB: names are cut, places are shed,
    /// and a path that alone cannot fit leaves the fallback line.
    #[test]
    fn every_body_fits_apns_limit() {
        let long = "x".repeat(900);
        let mut places: Vec<Place> = (0..7).map(|i| Place { repo: format!("/{long}{i}"), project: long.clone(), place: long.clone(), ..place("p") }).collect();
        let sev = Kind::Several { places: places.clone(), count: 7, waited_s: 30 };
        let b = body(&sev, 1, &long, &[1; 32], &[2; 8], [3; 12]);
        assert!(b.len() <= MAX_PAYLOAD, "{}", b.len());
        assert!(serde_json::from_slice::<Value>(&b).unwrap().get("wt").is_some(), "several's repos are not in its plaintext, so it fits sealed");
        let huge = "y".repeat(5000);
        places[0].repo = huge;
        let b = body(&Kind::Input { place: places[0].clone(), waited_s: 1 }, 1, "h", &[1; 32], &[2; 8], [3; 12]);
        let v: Value = serde_json::from_slice(&b).unwrap();
        assert!(b.len() <= MAX_PAYLOAD);
        assert_eq!(v.get("wt"), None, "too long to seal: the generic line alone");
        assert_eq!(v["aps"]["alert"]["body"], "A session needs input");
    }
}

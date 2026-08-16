//! BSD (macOS) vs GNU (Linux) `stat`/`date` handling, shelled out so it matches
//! the bash CLI byte-for-byte (`std::fs`/`chrono` would diverge — e.g. birth
//! time errs on many Linux filesystems). Probe once, then reuse.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

pub struct SysClock {
    stat_gnu: bool,
    date_gnu: bool,
}

fn out(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    if o.status.success() {
        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        None
    }
}

/// Birth epoch per (path, dev, ino). A directory's creation time never changes,
/// so this is a write-once memo — but the key carries dev+ino because
/// `rm foo && new foo` recreates the SAME path as a different directory, and a
/// path-only key would hand back the dead one's date forever.
/// `fs::metadata` is used ONLY to build the key; the emitted value still comes
/// from the shelled-out `stat`, so bash byte-compat is untouched.
#[allow(clippy::type_complexity)]
fn birth_cache() -> &'static Mutex<HashMap<(String, u64, u64), i64>> {
    static C: OnceLock<Mutex<HashMap<(String, u64, u64), i64>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Formatted date per epoch. `date` for a fixed instant is deterministic, and a
/// snapshot re-formats the same handful of epochs every poll.
/// Caveat: the format is LOCAL time, so a timezone change mid-run can leave a
/// cached date a day off until restart. Traded knowingly for one fewer spawn
/// per place per poll.
fn date_cache() -> &'static Mutex<HashMap<i64, String>> {
    static C: OnceLock<Mutex<HashMap<i64, String>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

impl SysClock {
    pub fn detect() -> Self {
        // Probed ONCE per process, not once per `Project::discover` — which is
        // once per snapshot per project, i.e. two spawns every poll forever.
        // Whether the host has BSD or GNU coreutils cannot change at runtime.
        static PROBE: OnceLock<(bool, bool)> = OnceLock::new();
        let &(stat_gnu, date_gnu) = PROBE.get_or_init(|| {
            // GNU `stat -c` succeeds; BSD `stat` rejects `-c`.
            let stat_gnu = Command::new("stat")
                .args(["-c", "%Y", "/"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            // GNU `date -d @0` prints 1970 (with -u); BSD `date` rejects `-d`.
            let date_gnu = out("date", &["-u", "-d", "@0", "+%Y"]).as_deref() == Some("1970");
            (stat_gnu, date_gnu)
        });
        SysClock { stat_gnu, date_gnu }
    }

    /// Birth (creation) epoch of a path; falls back to mtime when unknown; 0 if all fail.
    pub fn stat_birth(&self, path: &str) -> i64 {
        let key = std::fs::metadata(path).ok().map(|m| (path.to_string(), m.dev(), m.ino()));
        if let Some(k) = &key {
            if let Some(hit) = birth_cache().lock().ok().and_then(|c| c.get(k).copied()) {
                return hit;
            }
        }
        let epoch = self.stat_birth_uncached(path);
        // Don't memoize the failure sentinel. `stat_birth_uncached` returns 0 for
        // EVERY failure — including a transient fork failure under the parallel
        // snapshot fan-out — and the key stays valid for the life of the
        // directory, so a single unlucky spawn would pin `created: "-"` on that
        // place until the app restarts (and `recency_key` would sink it to the
        // bottom of the nav). Before the cache, the next 3s poll healed it.
        if epoch > 0 {
            if let Some(k) = key {
                if let Ok(mut c) = birth_cache().lock() {
                    c.insert(k, epoch);
                }
            }
        }
        epoch
    }

    fn stat_birth_uncached(&self, path: &str) -> i64 {
        if !self.stat_gnu {
            return out("stat", &["-f", "%B", path]).and_then(|s| s.parse().ok()).unwrap_or(0);
        }
        let w: i64 = out("stat", &["-c", "%W", path]).and_then(|s| s.parse().ok()).unwrap_or(0);
        if w > 0 {
            w
        } else {
            out("stat", &["-c", "%Y", path]).and_then(|s| s.parse().ok()).unwrap_or(0)
        }
    }

    /// Epoch → `YYYY-MM-DD`, or `-` when unknown.
    pub fn fmt_date(&self, epoch: i64) -> String {
        if epoch <= 0 {
            return "-".to_string();
        }
        if let Some(hit) = date_cache().lock().ok().and_then(|c| c.get(&epoch).cloned()) {
            return hit;
        }
        let res = if self.date_gnu {
            out("date", &["-d", &format!("@{epoch}"), "+%F"])
        } else {
            out("date", &["-r", &epoch.to_string(), "+%F"])
        };
        let s = res.unwrap_or_else(|| "-".to_string());
        // Don't memoize the failure sentinel — a transient `date` failure would
        // otherwise pin "-" for this epoch for the life of the process.
        if s != "-" {
            if let Ok(mut c) = date_cache().lock() {
                c.insert(epoch, s.clone());
            }
        }
        s
    }

    /// Compact age of `epoch` vs `now` (e.g. `3h`, `5d`, `2w`). Pure arithmetic.
    pub fn ago(&self, epoch: i64, now: i64) -> String {
        let mut s = now - epoch;
        if s < 0 {
            s = 0;
        }
        if s < 3600 {
            format!("{}m", s / 60)
        } else if s < 86400 {
            format!("{}h", s / 3600)
        } else if s < 604800 {
            format!("{}d", s / 86400)
        } else {
            format!("{}w", s / 604800)
        }
    }
}

/// `date +%Y%m%d-%H%M%S` — the backup-directory stamp `sync` names its
/// `--backup-dir` after. Shelled out for this module's usual reason (bats can
/// shim `date`); the format is identical on BSD and GNU, so there is no probe.
/// Falls back to the epoch, so a backup directory is never nameless.
pub fn stamp_now() -> String {
    out("date", &["+%Y%m%d-%H%M%S"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| now_epoch().to_string())
}

pub fn now_epoch() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago_buckets() {
        let c = SysClock { stat_gnu: false, date_gnu: false };
        assert_eq!(c.ago(1000, 1000), "0m");
        assert_eq!(c.ago(1000, 1000 + 120), "2m");
        assert_eq!(c.ago(0, 3 * 3600), "3h");
        assert_eq!(c.ago(0, 5 * 86400), "5d");
        assert_eq!(c.ago(0, 2 * 604800), "2w");
        assert_eq!(c.ago(1000, 500), "0m"); // clamp negative
    }

    #[test]
    fn stamp_is_shaped_like_a_directory_name() {
        let s = stamp_now();
        assert_eq!(s.len(), 15, "got {s}");
        assert_eq!(&s[8..9], "-");
        assert!(s.chars().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit()), "got {s}");
    }

    #[test]
    fn detect_and_fmt_roundtrip() {
        let c = SysClock::detect();
        // epoch 0 → "-", and a real epoch formats as YYYY-MM-DD on this host.
        assert_eq!(c.fmt_date(0), "-");
        let d = c.fmt_date(1_700_000_000);
        assert!(d.len() == 10 && d.as_bytes()[4] == b'-', "got {d}");
    }
}

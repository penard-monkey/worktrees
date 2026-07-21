//! BSD (macOS) vs GNU (Linux) `stat`/`date` handling, shelled out so it matches
//! the bash CLI byte-for-byte (`std::fs`/`chrono` would diverge — e.g. birth
//! time errs on many Linux filesystems). Probe once, then reuse.

use std::process::Command;

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

impl SysClock {
    pub fn detect() -> Self {
        // GNU `stat -c` succeeds; BSD `stat` rejects `-c`.
        let stat_gnu = Command::new("stat")
            .args(["-c", "%Y", "/"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        // GNU `date -d @0` prints 1970 (with -u); BSD `date` rejects `-d`.
        let date_gnu = out("date", &["-u", "-d", "@0", "+%Y"]).as_deref() == Some("1970");
        SysClock { stat_gnu, date_gnu }
    }

    /// Birth (creation) epoch of a path; falls back to mtime when unknown; 0 if all fail.
    pub fn stat_birth(&self, path: &str) -> i64 {
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
        let res = if self.date_gnu {
            out("date", &["-d", &format!("@{epoch}"), "+%F"])
        } else {
            out("date", &["-r", &epoch.to_string(), "+%F"])
        };
        res.unwrap_or_else(|| "-".to_string())
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
    fn detect_and_fmt_roundtrip() {
        let c = SysClock::detect();
        // epoch 0 → "-", and a real epoch formats as YYYY-MM-DD on this host.
        assert_eq!(c.fmt_date(0), "-");
        let d = c.fmt_date(1_700_000_000);
        assert!(d.len() == 10 && d.as_bytes()[4] == b'-', "got {d}");
    }
}

//! Read the executable as data, never run a replacement (ADR 0001).
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    io::{BufReader, Read},
    path::PathBuf,
};

// Used by --version so the linker retains the complete marker in release builds.
const RECORD: &str = concat!("\0WORKTREES_CLI_VERSION=", env!("CARGO_PKG_VERSION"), "\0");
pub fn version() -> &'static str {
    std::hint::black_box(RECORD)
        .trim_matches('\0')
        .strip_prefix("WORKTREES_CLI_VERSION=")
        .unwrap()
}

#[derive(Default)]
pub struct Stale {
    path: Option<PathBuf>,
    cached: Option<((u64, u64, u64, i64, i64), Option<String>)>,
}
impl Stale {
    pub fn current() -> Self {
        Self {
            path: std::env::current_exe().ok(),
            cached: None,
        }
    }
    pub fn warning(&mut self) -> Option<String> {
        let path = self.path.as_ref()?;
        let m = fs::metadata(path).ok()?;
        let stamp = (m.dev(), m.ino(), m.len(), m.ctime(), m.ctime_nsec());
        if self.cached.as_ref().map(|c| c.0) != Some(stamp) {
            let installed = fs::File::open(path).ok().and_then(read_version);
            self.cached = Some((stamp, installed));
        }
        let installed = self.cached.as_ref()?.1.as_deref()?;
        (installed != version()).then(|| format!(
            "This session's worktrees server is v{}, the installed binary is v{installed}; reload the MCP server to use the installed version. Client-cached tool definitions may survive reconnect; schema refresh after a full session restart is unverified.", version()
        ))
    }
}

// Stream with bounded memory and a 128 MiB ceiling. Scan only on a metadata
// change. Missing/unreadable/older binaries without a marker mean unknown,
// not stale; no PATH lookup, subprocess, network or repo metadata is involved.
fn read_version(file: fs::File) -> Option<String> {
    let prefix = b"\0WORKTREES_CLI_VERSION=";
    let mut reader = BufReader::new(file.take(128 * 1024 * 1024));
    let mut buf = [0u8; 65536];
    let mut matched = 0;
    let mut value = Vec::new();
    loop {
        let n = reader.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        for &b in &buf[..n] {
            if matched == prefix.len() {
                if b == 0 {
                    if let Ok(version) = std::str::from_utf8(&value) {
                        // The scanner's own prefix also occurs in the binary.
                        // Do not mistake adjacent unrelated string data for a
                        // version, and keep looking after an invalid candidate.
                        let core = version.split(['-', '+']).next().unwrap_or("");
                        let parts: Vec<_> = core.split('.').collect();
                        if parts.len() == 3
                            && parts
                                .iter()
                                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                        {
                            return Some(version.to_string());
                        }
                    }
                    value.clear();
                    matched = 1;
                } else if value.len() < 64 && (b.is_ascii_alphanumeric() || b".-+".contains(&b)) {
                    value.push(b);
                } else {
                    matched = 0;
                    value.clear();
                }
            } else if b == prefix[matched] {
                matched += 1;
            } else {
                matched = usize::from(b == 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_replacement_without_executing_it_and_stays_quiet_on_match() {
        let path = std::env::temp_dir().join(format!("worktrees-stale-{}", std::process::id()));
        fs::write(&path, RECORD).unwrap();
        let mut probe = Stale {
            path: Some(path.clone()),
            cached: None,
        };
        assert_eq!(probe.warning(), None);
        let replacement = path.with_extension("new");
        fs::write(
            &replacement,
            b"#!/bin/sh\nexit 99\n\0WORKTREES_CLI_VERSION=unrelated-data\0\0WORKTREES_CLI_VERSION=99.8.7\0",
        )
        .unwrap();
        fs::rename(&replacement, &path).unwrap();
        let warning = probe.warning().unwrap();
        assert!(warning.contains(&format!("server is v{}", version())));
        assert!(warning.contains("installed binary is v99.8.7"));
        assert!(warning.contains("full session restart is unverified"));
        assert_eq!(probe.warning().as_deref(), Some(warning.as_str()));
        fs::write(&path, b"old binary without marker").unwrap();
        assert_eq!(probe.warning(), None);
        fs::remove_file(&path).unwrap();
        assert_eq!(probe.warning(), None);
    }
}

//! `<config_dir>/serve-remote.json` — the host's opt-in to remote access
//! (Phase 2 §4.1): the address the host chose, its port, its interface, the
//! network fingerprint recorded at enable, and whether the host may be found
//! over mDNS (§4.7).
//!
//! Written only by a console act (`remote_enable`) and by the address watch
//! acting on it (a move, a learned fingerprint, the port the first bind got),
//! never by the frontend and never a project-config key — the reasoning that
//! keeps `--allow-origin` a flag (`policy.rs`). Mode `0600` through
//! `fsx::write_atomic` under `serve-remote.lock`. Off is `on: false` (or no
//! file): turning remote access off keeps the record, so turning it on again
//! reuses its PORT — a paired device stores `addr:port`, and a kill switch
//! must not cost every device its address.
//!
//! Read strictly: an unknown key, a newer version, an address the classifier
//! refuses or a value the std parsers do not take makes the file UNREADABLE.
//! Then remote access stays off, the status says why, and nothing rewrites
//! the file but the host enabling again.

use std::fs;
use std::net::IpAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use worktrees_core::netclass;

use super::super::netwatch::{Fingerprint, LinkAddr, Record, Router};
use super::super::servefile;

pub const FILE: &str = "serve-remote.json";

/// The format this build reads and writes.
pub const VERSION: u32 = 1;

/// The opt-in as serve holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// The bind the host chose; `None` only while off and never on — when the
    /// host has set its advert switch before remote access was first on.
    pub record: Option<Record>,
    /// 0 until the first bind has chosen one; then that one, for good, since a
    /// paired device stores `addr:port`.
    pub port: u16,
    /// §4.7 D6: the console's switch for the advert. On at every enable.
    pub advertise: bool,
    /// Remote access is on. Off keeps the rest, for the port.
    pub on: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    addr: Option<String>,
    port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    iface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    network: Option<Network>,
    advertise: bool,
    on: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Network {
    P2p { addr: String, prefix: u8 },
    Lan { iface: String, net: String, prefix: u8, router: Option<RouterOnDisk> },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouterOnDisk {
    ip: String,
    lladdr: Option<String>,
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// An address literal through std's strict parser — no shorthand, no hex or
/// octal octets, no zone (§4.1).
fn ip(s: &str) -> Result<IpAddr, String> {
    s.parse::<IpAddr>().map_err(|_| format!("{s:?} is not an address"))
}

fn to_disk(s: &Saved) -> OnDisk {
    let network = |r: &Record| match &r.fingerprint {
        Fingerprint::PointToPoint { addr, prefix } => Network::P2p { addr: addr.to_string(), prefix: *prefix },
        Fingerprint::Lan { iface, net, prefix, router } => Network::Lan {
            iface: iface.clone(),
            net: net.to_string(),
            prefix: *prefix,
            router: router.as_ref().map(|r| RouterOnDisk { ip: r.ip.to_string(), lladdr: r.lladdr.as_ref().map(LinkAddr::to_string) }),
        },
    };
    let r = s.record.as_ref();
    OnDisk {
        version: VERSION,
        addr: r.map(|r| r.addr.to_string()),
        port: s.port,
        iface: r.map(|r| r.iface.clone()),
        network: r.map(network),
        advertise: s.advertise,
        on: s.on,
    }
}

fn from_disk(d: OnDisk) -> Result<Saved, String> {
    if d.version != VERSION {
        return Err(format!("version {} (this build reads {VERSION})", d.version));
    }
    let (addr, iface, network) = match (d.addr, d.iface, d.network) {
        (Some(a), Some(i), Some(n)) => (a, i, n),
        (None, None, None) if !d.on => return Ok(Saved { record: None, port: d.port, advertise: d.advertise, on: false }),
        _ => return Err("an opt-in names an address, its interface and its network together, and an opt-in that is on names all three".into()),
    };
    let addr = ip(&addr)?;
    netclass::classify(addr).map_err(|e| e.to_string())?;
    let fingerprint = match network {
        Network::P2p { addr: a, prefix } => Fingerprint::PointToPoint { addr: ip(&a)?, prefix },
        Network::Lan { iface, net, prefix, router } => {
            let router = match router {
                None => None,
                Some(r) => {
                    let lladdr = match r.lladdr {
                        None => None,
                        Some(l) => Some(LinkAddr::parse(&l).ok_or_else(|| format!("{l:?} is not a link-layer address"))?),
                    };
                    Some(Router { ip: ip(&r.ip)?, lladdr })
                }
            };
            netclass::subnet_row(ip(&net)?, prefix).map_err(|e| e.to_string())?;
            Fingerprint::Lan { iface, net: ip(&net)?, prefix, router }
        }
    };
    Ok(Saved { record: Some(Record { addr, iface, fingerprint }), port: d.port, advertise: d.advertise, on: d.on })
}

/// The opt-in: `Ok(None)` when remote access is off (no file), `Err` when the
/// file is there and cannot be read as one.
pub fn read(dir: &Path) -> Result<Option<Saved>, String> {
    let p = path(dir);
    let m = match fs::symlink_metadata(&p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", p.display())),
        Ok(m) => m,
    };
    if !m.file_type().is_file() {
        return Err(format!("{} is not a regular file", p.display()));
    }
    let bytes = fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let d: OnDisk = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", p.display()))?;
    from_disk(d).map(Some).map_err(|e| format!("{}: {e}", p.display()))
}

pub fn write(dir: &Path, s: &Saved) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&to_disk(s)).map_err(|e| e.to_string())? + "\n";
    servefile::with_lock(dir, "serve-remote", || {
        worktrees_core::fsx::write_atomic(&path(dir), json.as_bytes(), Some(fs::Permissions::from_mode(0o600)))
            .map_err(|e| format!("{}: {e}", path(dir).display()))
    })
}

/// Off: the file goes. A file that is not there is already off.
pub fn remove(dir: &Path) -> Result<(), String> {
    servefile::with_lock(dir, "serve-remote", || match fs::remove_file(path(dir)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path(dir).display())),
        _ => Ok(()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-remote-file-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn lan() -> Saved {
        Saved {
            record: Some(Record {
                addr: "192.168.1.5".parse().unwrap(),
                iface: "en0".into(),
                fingerprint: Fingerprint::Lan {
                    iface: "en0".into(),
                    net: "192.168.1.0".parse().unwrap(),
                    prefix: 24,
                    router: Some(Router { ip: "192.168.1.1".parse().unwrap(), lladdr: LinkAddr::parse("d8:44:89:a:b:c") }),
                },
            }),
            port: 51234,
            advertise: true,
            on: true,
        }
    }

    #[test]
    fn the_opt_in_round_trips_owner_only() {
        let d = dir("round");
        assert_eq!(read(&d), Ok(None), "no file is off");
        write(&d, &lan()).unwrap();
        assert_eq!(read(&d).unwrap(), Some(lan()));
        assert_eq!(fs::metadata(path(&d)).unwrap().permissions().mode() & 0o777, 0o600);
        let p2p = Saved {
            record: Some(Record {
                addr: "100.101.7.3".parse().unwrap(),
                iface: "utun6".into(),
                fingerprint: Fingerprint::PointToPoint { addr: "100.101.7.3".parse().unwrap(), prefix: 32 },
            }),
            port: 7443,
            advertise: false,
            on: false,
        };
        write(&d, &p2p).unwrap();
        assert_eq!(read(&d).unwrap(), Some(p2p));
        // A preference set before remote access was ever on: no bind at all.
        let pref = Saved { record: None, port: 0, advertise: false, on: false };
        write(&d, &pref).unwrap();
        assert_eq!(read(&d).unwrap(), Some(pref));
        remove(&d).unwrap();
        assert_eq!(read(&d), Ok(None));
        remove(&d).unwrap();
        let _ = fs::remove_dir_all(&d);
    }

    /// Unreadable, never repaired: an unknown key, a newer version, a public
    /// or loopback address, a spelling std does not take, a subnet wider than
    /// its row, and something that is not a regular file.
    #[test]
    fn a_file_that_is_not_exactly_an_opt_in_is_unreadable() {
        let d = dir("strict");
        write(&d, &lan()).unwrap();
        let good: serde_json::Value = serde_json::from_slice(&fs::read(path(&d)).unwrap()).unwrap();
        let bad = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut v = good.clone();
            f(&mut v);
            fs::write(path(&d), v.to_string()).unwrap();
            read(&d)
        };
        assert!(bad(&|v| v["extra"] = 1.into()).is_err(), "unknown key");
        assert!(bad(&|v| v["version"] = 2.into()).is_err(), "newer version");
        assert!(bad(&|v| v["addr"] = "8.8.8.8".into()).is_err(), "public");
        assert!(bad(&|v| v["addr"] = "127.0.0.1".into()).is_err(), "loopback");
        assert!(bad(&|v| v["addr"] = "192.168.001.5".into()).is_err(), "a leading zero");
        assert!(bad(&|v| v["network"]["lan"]["prefix"] = 7.into()).is_err(), "wider than its row");
        assert!(bad(&|v| v["network"]["lan"]["router"]["lladdr"] = "nope".into()).is_err(), "link-layer");
        assert!(bad(&|v| { v.as_object_mut().unwrap().remove("network"); }).is_err(), "half a bind");
        assert!(bad(&|v| { for k in ["addr", "iface", "network"] { v.as_object_mut().unwrap().remove(k); } }).is_err(), "on with no bind");
        assert!(bad(&|_| {}).is_ok(), "and the untouched file reads");
        fs::remove_file(path(&d)).unwrap();
        std::os::unix::fs::symlink(d.join("elsewhere"), path(&d)).unwrap();
        fs::write(d.join("elsewhere"), good.to_string()).unwrap();
        assert!(read(&d).is_err(), "a link at the name is not followed");
        let _ = fs::remove_dir_all(&d);
    }
}

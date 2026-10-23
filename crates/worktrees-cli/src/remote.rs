//! `worktrees remote …`: this machine as a paired DEVICE of another host's
//! `worktrees serve` (Phase 2 §2.2, §4.4) — the fastest end-to-end test of
//! the remote listener, and the CI witness's client. Everything goes through
//! `worktrees_remote`, the same transport the desktop app's shim and the iOS
//! app dial through.
//!
//! The hosts live in the desktop app's config dir (`hostenv::default_config_dir`)
//! unless `--config-dir` says otherwise, so a host paired here is the app's too.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};
use worktrees_remote::call::{self, CallError};
use worktrees_remote::hosts::HostStore;
use worktrees_remote::pair::{self, PairEvent};

pub const USAGE: &str = "\
usage: worktrees remote pair <link> [--name NAME]          pair with a host from its pairing link
       worktrees remote hosts [--json]                     the hosts this machine is paired with
       worktrees remote call <host> <cmd> [JSON-ARGS] [--timeout S]
                                                           run one request on a host and print its answer
       worktrees remote forget <host>                      forget a host and delete this machine's key for it

  <host> is a host's id (h_…) or its name, as `worktrees remote hosts` lists them.
  --config-dir DIR  where the hosts are kept (default: the desktop app's config dir)

The link is the `worktrees-pair:v1?…` text the host shows beside its pairing QR.
Its address is checked before anything is dialled: only a private address
(a LAN, Tailscale, a cable) is accepted.";

struct Opts {
    dir: PathBuf,
    name: Option<String>,
    json: bool,
    timeout: Duration,
    rest: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { dir: worktrees_api::hostenv::default_config_dir(), name: None, json: false, timeout: Duration::from_secs(30), rest: Vec::new() };
    let mut it = args.iter();
    let mut positional = false;
    while let Some(a) = it.next() {
        if positional {
            o.rest.push(a.clone());
            continue;
        }
        match a.as_str() {
            "--" => positional = true,
            "--config-dir" => o.dir = PathBuf::from(it.next().ok_or("--config-dir needs a directory")?),
            "--name" => o.name = Some(it.next().ok_or("--name needs a name")?.clone()),
            "--json" => o.json = true,
            "--timeout" => {
                let v = it.next().ok_or("--timeout needs seconds")?;
                let s: u64 = v.parse().ok().filter(|s| (1..=3600).contains(s)).ok_or_else(|| format!("--timeout {v:?}: 1 to 3600 seconds"))?;
                o.timeout = Duration::from_secs(s);
            }
            "-h" | "--help" => return Err(String::new()),
            f if f.starts_with('-') && f.len() > 1 => return Err(format!("unknown flag {f}")),
            _ => o.rest.push(a.clone()),
        }
    }
    if o.dir.as_os_str().is_empty() {
        return Err("--config-dir: empty".into());
    }
    Ok(o)
}

pub fn cmd_remote(args: &[String]) -> i32 {
    let (verb, rest) = match args.split_first() {
        Some((v, r)) => (v.as_str(), r),
        None => {
            println!("{USAGE}");
            return 2;
        }
    };
    if matches!(verb, "-h" | "--help" | "help") {
        println!("{USAGE}");
        return 0;
    }
    let o = match parse(rest) {
        Ok(o) => o,
        Err(e) if e.is_empty() => {
            println!("{USAGE}");
            return 0;
        }
        Err(e) => {
            eprintln!("worktrees remote: {e}\n\n{USAGE}");
            return 2;
        }
    };
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("worktrees remote: {e}");
            return 1;
        }
    };
    let store = HostStore::new(&o.dir);
    match (verb, o.rest.as_slice()) {
        ("pair", [link]) => rt.block_on(do_pair(&store, link, o.name.as_deref())),
        ("hosts", []) => hosts(&store, o.json),
        ("call", [host, cmd]) => rt.block_on(do_call(&store, host, cmd, "{}", o.timeout)),
        ("call", [host, cmd, a]) => rt.block_on(do_call(&store, host, cmd, a, o.timeout)),
        ("forget", [host]) => forget(&store, host),
        ("pair" | "hosts" | "call" | "forget", _) => {
            eprintln!("worktrees remote {verb}: wrong arguments\n\n{USAGE}");
            2
        }
        _ => {
            eprintln!("worktrees remote: unknown verb {verb:?}\n\n{USAGE}");
            2
        }
    }
}

async fn do_pair(store: &HostStore, link: &str, name: Option<&str>) -> i32 {
    let name = name.map(str::to_string).unwrap_or_else(pair::this_name);
    let mut host = String::from("the host");
    let mut on = |ev: PairEvent| match ev {
        PairEvent::Dialling { host: h, addr } => {
            eprintln!("Pairing with {h} at {addr} as {name:?}…");
            host = h;
        }
        PairEvent::Code(c) => {
            println!("Comparison code: {} {}", &c[..3], &c[3..]);
            eprintln!("Check that {host} shows this same code, then press Allow there. Waiting…");
        }
    };
    match pair::pair(store, link, &name, worktrees_core::sysclock::now_epoch(), &mut on).await {
        Ok(rec) => {
            println!("Paired with {} ({}), as device {}.", rec.name, rec.addr, rec.device_id);
            println!("Try: worktrees remote call {} list_workspace", rec.id);
            0
        }
        Err(e) => {
            eprintln!("worktrees remote pair: {e}");
            1
        }
    }
}

fn hosts(store: &HostStore, as_json: bool) -> i32 {
    let h = match store.list() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("worktrees remote hosts: {e}");
            return 1;
        }
    };
    if as_json {
        let rows: Vec<Value> = h.hosts.iter().map(|r| json!({"id": r.id, "name": r.name, "addr": r.addr, "pin": r.pin, "device_id": r.device_id, "expiry": r.expiry, "paired_at": r.paired_at})).collect();
        println!("{}", json!({"chosen": h.chosen, "hosts": rows}));
        return 0;
    }
    if h.hosts.is_empty() {
        println!("No paired hosts. Pair with one: worktrees remote pair '<link>'");
        return 0;
    }
    for r in &h.hosts {
        let mark = if h.chosen.as_deref() == Some(r.id.as_str()) { "*" } else { " " };
        println!("{mark} {}  {:<24}  {:<22}  device {}", r.id, r.name, r.addr, r.device_id);
    }
    if h.chosen.is_some() {
        println!("(* is the host the desktop app's window drives)");
    }
    0
}

async fn do_call(store: &HostStore, sel: &str, cmd: &str, args: &str, wait: Duration) -> i32 {
    let rec = match store.get(sel) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("worktrees remote call: {e}");
            return 2;
        }
    };
    let args: Value = match serde_json::from_str(args) {
        Ok(v @ Value::Object(_)) => v,
        _ => {
            eprintln!("worktrees remote call: the arguments are one JSON object, e.g. '{{\"repo\": \"/path\"}}'");
            return 2;
        }
    };
    let key = match store.key_for(&rec) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("worktrees remote call: {e}");
            return 1;
        }
    };
    match call::call(&rec, &key, env!("CARGO_PKG_VERSION"), cmd, args, wait).await {
        Ok((_, v)) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()));
            0
        }
        Err(CallError::Refused(v)) => {
            eprintln!("{}: {}", cmd, v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
            1
        }
        Err(e) => {
            eprintln!("worktrees remote call: {} ({}): {e}", rec.name, rec.addr);
            1
        }
    }
}

fn forget(store: &HostStore, sel: &str) -> i32 {
    match store.forget(sel) {
        Ok(r) => {
            println!(
                "Forgot {} ({}) and deleted this machine's key for it. {} still lists this device ({}) until it is revoked there.",
                r.name, r.id, r.name, r.device_id
            );
            0
        }
        Err(e) => {
            eprintln!("worktrees remote forget: {e}");
            1
        }
    }
}

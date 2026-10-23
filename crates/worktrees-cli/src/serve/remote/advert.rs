//! The mDNS advert's backends (Phase 2 §4.7): serve asks the OS's OWN
//! responder, so serve parses no multicast traffic and runs no UDP listener
//! of its own.
//!
//! - **macOS: `dns_sd`** (libSystem; no crate). On one connection
//!   (`DNSServiceCreateConnection`): `DNSServiceRegisterRecord` for the host
//!   name `wt-<id>.local` with ONE address record, the bound address, on the
//!   bound interface, `kDNSServiceFlagsUnique`; then `DNSServiceRegister` of
//!   `_worktrees._tcp` on the same interface with `host` set to that name.
//!   `dns_sd.h` says a host named there gets no address record by itself,
//!   which is why the record is registered first. `NoAutoRename`: a name
//!   another host already holds is reported, never renamed (§4.7). Measured
//!   2026-10-22, macOS 27, through `dns-sd -lo -P` (the same two calls) on
//!   the local-only interface: the name resolves to the one address given
//!   and to no AAAA; the SRV names it; the TXT is exactly `v=1 id=…`.
//!   Deallocating the connection removes both records.
//! - Elsewhere: none yet (`super::NoAdvertiser`).
//!
//! A thread owns the connection, waits on its socket and on a self-pipe, and
//! is the only thread that touches it, so a withdraw never races a callback:
//! the withdraw writes to the pipe and joins the thread, which deallocates.

/// The TXT record (§4.7): exactly `v=1` and `id=<discovery id>`, each a
/// length-prefixed string. Built from constants and the id alone.
pub fn txt(id: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for kv in ["v=1".to_string(), format!("id={id}")] {
        out.push(kv.len() as u8);
        out.extend_from_slice(kv.as_bytes());
    }
    out
}

/// The host name the SRV names: serve's own, from the discovery id alone, so
/// it reveals nothing the TXT does not (§4.7, §12.4 T1).
pub fn host_name(id: &str) -> String {
    format!("wt-{id}.local")
}

#[cfg(target_os = "macos")]
pub use macos::DnsSd;

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void, CString};
    use std::net::IpAddr;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::super::{Advert, Advertiser};

    #[repr(C)]
    struct Opaque {
        _p: [u8; 0],
    }
    type DNSServiceRef = *mut Opaque;
    type DNSRecordRef = *mut Opaque;
    type RegisterReply = extern "C" fn(DNSServiceRef, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
    type RecordReply = extern "C" fn(DNSServiceRef, DNSRecordRef, u32, i32, *mut c_void);

    extern "C" {
        fn DNSServiceCreateConnection(sd: *mut DNSServiceRef) -> i32;
        #[allow(clippy::too_many_arguments)]
        fn DNSServiceRegisterRecord(
            sd: DNSServiceRef,
            rec: *mut DNSRecordRef,
            flags: u32,
            iface: u32,
            fullname: *const c_char,
            rrtype: u16,
            rrclass: u16,
            rdlen: u16,
            rdata: *const c_void,
            ttl: u32,
            cb: RecordReply,
            ctx: *mut c_void,
        ) -> i32;
        #[allow(clippy::too_many_arguments)]
        fn DNSServiceRegister(
            sd: *mut DNSServiceRef,
            flags: u32,
            iface: u32,
            name: *const c_char,
            regtype: *const c_char,
            domain: *const c_char,
            host: *const c_char,
            port: u16,
            txt_len: u16,
            txt: *const c_void,
            cb: RegisterReply,
            ctx: *mut c_void,
        ) -> i32;
        fn DNSServiceRefSockFD(sd: DNSServiceRef) -> i32;
        fn DNSServiceProcessResult(sd: DNSServiceRef) -> i32;
        fn DNSServiceRefDeallocate(sd: DNSServiceRef);
    }

    const NO_AUTO_RENAME: u32 = 0x8;
    const UNIQUE: u32 = 0x20;
    const SHARE_CONNECTION: u32 = 0x4000;
    const CLASS_IN: u16 = 1;
    const TYPE_A: u16 = 1;
    const TYPE_AAAA: u16 = 28;
    const NAME_CONFLICT: i32 = -65548;
    /// The address record's TTL: mDNS's own for host records.
    const TTL: u32 = 120;
    /// How long `register` waits for the responder's first verdict. Probing
    /// takes about three quarters of a second on a real link; past this the
    /// advert counts as up and a later conflict is reported by `trouble`.
    const FIRST_VERDICT: Duration = Duration::from_secs(3);

    fn describe(code: i32) -> String {
        match code {
            NAME_CONFLICT => "another host on this link already holds the name; not advertised this time (the address code still works)".into(),
            c => format!("mDNSResponder refused the registration (dns_sd error {c})"),
        }
    }

    /// What the callbacks share with `register`, on the connection's thread.
    struct Ctx {
        first: Option<mpsc::Sender<Result<(), String>>>,
        pending: u8,
        stop: bool,
        trouble: Arc<Mutex<Option<String>>>,
    }

    impl Ctx {
        fn outcome(&mut self, code: i32) {
            if code != 0 {
                let why = describe(code);
                *self.trouble.lock().unwrap() = Some(why.clone());
                self.stop = true;
                if let Some(tx) = self.first.take() {
                    let _ = tx.send(Err(why));
                }
                return;
            }
            self.pending = self.pending.saturating_sub(1);
            if self.pending == 0 {
                if let Some(tx) = self.first.take() {
                    let _ = tx.send(Ok(()));
                }
            }
        }
    }

    extern "C" fn on_record(_: DNSServiceRef, _: DNSRecordRef, _: u32, code: i32, ctx: *mut c_void) {
        // SAFETY: `ctx` is the `Ctx` the thread below owns and outlives the
        // connection; callbacks run only inside its `DNSServiceProcessResult`.
        unsafe { &mut *(ctx as *mut Ctx) }.outcome(code);
    }

    extern "C" fn on_service(_: DNSServiceRef, _: u32, code: i32, _: *const c_char, _: *const c_char, _: *const c_char, ctx: *mut c_void) {
        // SAFETY: as `on_record`.
        unsafe { &mut *(ctx as *mut Ctx) }.outcome(code);
    }

    struct Live {
        stop: OwnedFd,
        thread: std::thread::JoinHandle<()>,
    }

    /// The macOS backend: one registration at a time.
    #[derive(Default)]
    pub struct DnsSd {
        live: Mutex<Option<Live>>,
        trouble: Arc<Mutex<Option<String>>>,
    }

    fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
        let mut fds = [0; 2];
        // SAFETY: a two-int array, as pipe(2) takes.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: both fresh descriptors are ours from here on.
        let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        for fd in [&r, &w] {
            // SAFETY: fcntl on a descriptor we own.
            unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        Ok((r, w))
    }

    impl Advertiser for DnsSd {
        fn register(&self, ad: &Advert) -> Result<String, String> {
            self.withdraw();
            *self.trouble.lock().unwrap() = None;
            let host = super::host_name(&ad.id);
            let fqdn = CString::new(format!("{host}.")).map_err(|e| e.to_string())?;
            let name = CString::new(ad.id.clone()).map_err(|e| e.to_string())?;
            let txt = super::txt(&ad.id);
            let (rrtype, rdata): (u16, Vec<u8>) = match ad.ip {
                IpAddr::V4(v) => (TYPE_A, v.octets().to_vec()),
                IpAddr::V6(v) => (TYPE_AAAA, v.octets().to_vec()),
            };
            let (index, port) = (ad.index, ad.port);
            let (stop_r, stop_w) = pipe().map_err(|e| format!("mDNS: {e}"))?;
            let (tx, rx) = mpsc::channel();
            let trouble = self.trouble.clone();
            let thread = std::thread::Builder::new()
                .name("mdns-advert".into())
                .spawn(move || {
                    let mut ctx = Ctx { first: Some(tx), pending: 2, stop: false, trouble };
                    let cx = &mut ctx as *mut Ctx as *mut c_void;
                    let mut conn: DNSServiceRef = std::ptr::null_mut();
                    // SAFETY: the dns_sd calls as dns_sd.h documents them; every
                    // pointer handed over outlives the connection, which is
                    // deallocated on this thread before `ctx` and the strings drop.
                    unsafe {
                        let e = DNSServiceCreateConnection(&mut conn);
                        if e != 0 {
                            ctx.outcome(e);
                            return;
                        }
                        let mut rec: DNSRecordRef = std::ptr::null_mut();
                        let e = DNSServiceRegisterRecord(
                            conn, &mut rec, UNIQUE, index, fqdn.as_ptr(), rrtype, CLASS_IN, rdata.len() as u16,
                            rdata.as_ptr() as *const c_void, TTL, on_record, cx,
                        );
                        if e != 0 {
                            ctx.outcome(e);
                            DNSServiceRefDeallocate(conn);
                            return;
                        }
                        let mut svc = conn;
                        let e = DNSServiceRegister(
                            &mut svc, SHARE_CONNECTION | NO_AUTO_RENAME, index, name.as_ptr(), c"_worktrees._tcp".as_ptr(),
                            std::ptr::null(), fqdn.as_ptr(), port.to_be(), txt.len() as u16, txt.as_ptr() as *const c_void, on_service, cx,
                        );
                        if e != 0 {
                            ctx.outcome(e);
                            DNSServiceRefDeallocate(conn);
                            return;
                        }
                        let sock = DNSServiceRefSockFD(conn);
                        while !ctx.stop {
                            let mut fds = [
                                libc::pollfd { fd: sock, events: libc::POLLIN, revents: 0 },
                                libc::pollfd { fd: stop_r.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                            ];
                            if libc::poll(fds.as_mut_ptr(), 2, -1) < 0 {
                                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                                    continue;
                                }
                                break;
                            }
                            if fds[1].revents != 0 {
                                break;
                            }
                            if fds[0].revents != 0 && DNSServiceProcessResult(conn) != 0 {
                                break;
                            }
                        }
                        // Removes the service and the host record together.
                        DNSServiceRefDeallocate(conn);
                    }
                })
                .map_err(|e| format!("mDNS: {e}"))?;
            *self.live.lock().unwrap() = Some(Live { stop: stop_w, thread });
            match rx.recv_timeout(FIRST_VERDICT) {
                Ok(Ok(())) | Err(mpsc::RecvTimeoutError::Timeout) => Ok(host),
                Ok(Err(why)) => {
                    self.withdraw();
                    Err(why)
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.withdraw();
                    Err("mDNSResponder could not be reached".into())
                }
            }
        }

        fn withdraw(&self) {
            let Some(live) = self.live.lock().unwrap().take() else { return };
            // SAFETY: one byte into the pipe the thread polls.
            unsafe { libc::write(live.stop.as_raw_fd(), [1u8].as_ptr() as *const c_void, 1) };
            let _ = live.thread.join();
        }

        fn trouble(&self) -> Option<String> {
            self.trouble.lock().unwrap().clone()
        }
    }

    impl Drop for DnsSd {
        fn drop(&mut self) {
            self.withdraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §4.7, D2: exactly two keys, `v=1` and `id`, length-prefixed.
    #[test]
    fn the_txt_record_is_v_and_id_and_nothing_else() {
        let id = "abcdefghijklmnopqrstuvwxyz";
        let t = txt(id);
        assert_eq!(t[0], 3);
        assert_eq!(&t[1..4], b"v=1");
        assert_eq!(t[4] as usize, 3 + id.len());
        assert_eq!(&t[5..], format!("id={id}").as_bytes());
        assert_eq!(host_name(id), "wt-abcdefghijklmnopqrstuvwxyz.local");
        assert!(host_name(id).len() - ".local".len() <= 63, "a DNS label");
    }

    /// The real responder, on its LOCAL-ONLY interface (no packet leaves this
    /// machine): the host name resolves to the one address given and no
    /// other; the SRV names it with the port; the TXT is `v=1 id=…`; after a
    /// withdraw, nothing resolves. Read back with Apple's own `dns-sd`.
    #[cfg(target_os = "macos")]
    #[test]
    fn mdnsresponder_answers_the_advert_and_forgets_it() {
        use super::super::{Advert, Advertiser};
        const LOCAL_ONLY: u32 = u32::MAX;
        let id = format!("t{}", super::super::super::identity::new_discovery_id().split_at(25).0);
        let ad = Advert { id: id.clone(), ip: "10.99.0.5".parse().unwrap(), port: 7443, index: LOCAL_ONLY };
        let dnssd = DnsSd::default();
        let host = dnssd.register(&ad).expect("registered");
        assert_eq!(host, format!("wt-{id}.local"));
        let q = |args: &[&str]| {
            let o = std::process::Command::new("/usr/bin/dns-sd").args(["-lo", "-t", "2"]).args(args).output().unwrap();
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        let g = q(&["-G", "v4v6", &host]);
        assert!(g.contains("10.99.0.5"), "{g}");
        assert!(!g.lines().any(|l| l.contains("Add") && l.contains("::") && !l.contains("No Such Record")), "an AAAA answered: {g}");
        let l = q(&["-L", &id, "_worktrees._tcp", "local"]);
        assert!(l.contains(&format!("{host}.:7443")), "{l}");
        assert!(l.contains(&format!("v=1 id={id}")), "{l}");
        dnssd.withdraw();
        let g = q(&["-G", "v4", &host]);
        assert!(!g.contains("10.99.0.5"), "still answered after the withdraw: {g}");
    }
}

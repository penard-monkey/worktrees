// llprobe.rs — can a link-local address be dialed from a STRING, as a QR
// would carry it? What do Rust's std parsers accept (§4.1: "the bytes come
// from Rust's std::net parsers ... never from inet_aton or a resolver")?
//
//   llprobe parse                      table of spellings through std parsers
//                                      and through the resolver (ToSocketAddrs)
//   llprobe listen <addr[%zone]> <port>
//   llprobe dial <addr> <port> [zone]  zone: an interface NAME, resolved with
//                                      if_nametoindex; omitted = scope id 0
//
// std only, so it builds with a bare `rustc -O llprobe.rs`. Spike probe for
// docs/proposals/server-edition-phase2.md §4.1; not product code.
use std::ffi::CString;
use std::io::{Read, Write};
use std::net::*;
use std::os::raw::{c_char, c_uint};
use std::time::{Duration, Instant};

extern "C" {
    fn if_nametoindex(name: *const c_char) -> c_uint;
}

fn ifindex(name: &str) -> u32 {
    let c = CString::new(name).unwrap();
    unsafe { if_nametoindex(c.as_ptr()) }
}

fn show<T: std::fmt::Debug, E: std::fmt::Display>(r: Result<T, E>) -> String {
    match r {
        Ok(v) => format!("Ok({v:?})"),
        Err(e) => format!("Err({e})"),
    }
}

fn parse() {
    let ips = [
        "169.254.10.1", "fe80::1", "fe80::1%lo0", "fe80::1%1", "127.1", "0x7f.0.0.1",
        "010.0.0.1", "10.0.0.256", "::ffff:10.0.0.1", "::ffff:a00:1", " 10.0.0.1",
    ];
    for s in ips {
        println!("IpAddr     {s:<18} -> {}", show(s.parse::<IpAddr>()));
    }
    let socks = [
        "169.254.10.1:7000", "[fe80::1]:7000", "[fe80::1%lo0]:7000", "[fe80::1%1]:7000",
        "[fe80::1%25lo0]:7000", "fe80::1%lo0:7000", "[::ffff:127.0.0.1]:80",
    ];
    for s in socks {
        println!("SocketAddr {s:<22} -> {}", show(s.parse::<SocketAddr>()));
        println!("SockAddrV6 {s:<22} -> {}", show(s.parse::<SocketAddrV6>()));
    }
    // The resolver path: what `TcpStream::connect(&str)` and friends use when
    // the std parser refuses. getaddrinfo accepts every spelling §4.1 refuses.
    for (h, p) in [("fe80::1%lo0", 7000u16), ("127.1", 80), ("0x7f.0.0.1", 80), ("010.0.0.1", 80)] {
        let r: Result<Vec<SocketAddr>, _> = (h, p).to_socket_addrs().map(|i| i.collect());
        println!("resolver   ({h:?}, {p}) -> {}", show(r));
    }
    for s in ["[fe80::1%lo0]:7000", "127.1:80"] {
        let r: Result<Vec<SocketAddr>, _> = s.to_socket_addrs().map(|i| i.collect());
        println!("resolver   {s:?} -> {}", show(r));
    }
    let m: Ipv6Addr = "::ffff:10.0.0.1".parse().unwrap();
    println!("::ffff:10.0.0.1 to_ipv4_mapped() = {:?}, is_unicast_link_local(fe80::1) = {}",
        m.to_ipv4_mapped(), "fe80::1".parse::<Ipv6Addr>().unwrap().is_unicast_link_local());
}

fn split_zone(s: &str) -> (IpAddr, u32) {
    match s.split_once('%') {
        Some((a, z)) => (a.parse().expect("addr"), ifindex(z)),
        None => (s.parse().expect("addr"), 0),
    }
}

fn sockaddr(ip: IpAddr, port: u16, scope: u32) -> SocketAddr {
    match ip {
        IpAddr::V4(v4) => SocketAddr::V4(SocketAddrV4::new(v4, port)),
        IpAddr::V6(v6) => SocketAddr::V6(SocketAddrV6::new(v6, port, 0, scope)),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|s| s.as_str()) {
        Some("parse") => parse(),
        Some("listen") => {
            let (ip, scope) = split_zone(&a[2]);
            let sa = sockaddr(ip, a[3].parse().unwrap(), scope);
            let l = TcpListener::bind(sa).expect("bind");
            println!("listening on {:?}", l.local_addr().unwrap());
            for c in l.incoming() {
                let mut c = c.unwrap();
                println!("accepted {:?}", c.peer_addr().unwrap());
                let _ = c.write_all(b"hi\n");
            }
        }
        Some("dial") => {
            let ip: IpAddr = a[2].parse().expect("addr");
            let scope = a.get(4).map(|z| ifindex(z)).unwrap_or(0);
            let sa = sockaddr(ip, a[3].parse().unwrap(), scope);
            let t = Instant::now();
            match TcpStream::connect_timeout(&sa, Duration::from_secs(3)) {
                Ok(mut s) => {
                    let mut b = [0u8; 3];
                    let _ = s.read(&mut b);
                    println!("dial {sa} (scope_id {scope}) -> OK, local {:?}", s.local_addr().unwrap())
                }
                Err(e) => println!("dial {sa} (scope_id {scope}) -> Err({e}; raw_os_error={:?}) after {:.2}s",
                    e.raw_os_error(), t.elapsed().as_secs_f32()),
            }
        }
        _ => eprintln!("usage: llprobe parse | listen <addr[%zone]> <port> | dial <addr> <port> [zone]"),
    }
}

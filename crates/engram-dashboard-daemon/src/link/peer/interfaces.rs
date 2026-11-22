//! "My address" — this PC's direct `ip:port` addresses another PC can type into its trusted entry for this one: each address of each
//!   network interface that is up, with the port the endpoint listens on. Relay and lookup paths need no typing and are not listed.
//!
//! Kept: IPv4 other than loopback, link-local (169.254/16) and unspecified; IPv6 global unicast only (2000::/3 — no loopback,
//!   link-local or unique-local). A VPN's or a second LAN's address is listed beside the main one — which one the other PC can reach is
//!   only known there. The interfaces come from `netdev` (already in the graph through the link adapter); the choice of addresses is
//!   [`my_addresses`], pure.

// ADR-0389

use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use engram_dashboard_base::sync::lock;

/// How long a read of the interfaces is reused — the read can take seconds (netdev looks up each gateway's hardware address).
const INTERFACES_TTL: Duration = Duration::from_secs(30);

/// The last read of the interfaces and when it was made.
static INTERFACES: Mutex<Option<(Instant, Vec<IpAddr>)>> = Mutex::new(None);

/// [`interface_ips`], reused for [`INTERFACES_TTL`]. ★Blocking★ — call off the async workers (`peer.list` runs on the blocking pool).
pub(crate) fn cached_interface_ips() -> Vec<IpAddr> {
    cached(&INTERFACES, Instant::now(), interface_ips)
}

/// The cache rule: a read younger than [`INTERFACES_TTL`] is reused, else `read` runs (outside the lock — two callers may both read).
fn cached(
    cache: &Mutex<Option<(Instant, Vec<IpAddr>)>>,
    now: Instant,
    read: impl FnOnce() -> Vec<IpAddr>,
) -> Vec<IpAddr> {
    if let Some((at, ips)) = lock(cache).as_ref() {
        if now.saturating_duration_since(*at) < INTERFACES_TTL {
            return ips.clone();
        }
    }
    let ips = read();
    *lock(cache) = Some((now, ips.clone()));
    ips
}

/// The addresses of the interfaces that are up (unfiltered). An OS failure gives none (netdev reports no error).
pub(crate) fn interface_ips() -> Vec<IpAddr> {
    netdev::get_interfaces()
        .into_iter()
        .filter(|interface| interface.is_up() && !interface.is_loopback())
        .flat_map(|interface| {
            let v4 = interface.ipv4.iter().map(|net| IpAddr::V4(net.addr()));
            let v6 = interface.ipv6.iter().map(|net| IpAddr::V6(net.addr()));
            v4.chain(v6).collect::<Vec<_>>()
        })
        .collect()
}

/// Whether another PC could dial `ip` directly.
pub(crate) fn usable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !(v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xe000) == 0x2000,
    }
}

/// The endpoint's bound sockets → the addresses to show: a socket bound to one address as it is (if usable); one bound to every
///   interface of its family as each usable interface address of that family with its port. Sorted IPv4 first, no duplicates.
pub(crate) fn my_addresses(bound: &[SocketAddr], interfaces: &[IpAddr]) -> Vec<String> {
    let mut out: Vec<SocketAddr> = Vec::new();
    for socket in bound {
        if socket.ip().is_unspecified() {
            for ip in interfaces
                .iter()
                .filter(|ip| ip.is_ipv4() == socket.is_ipv4() && usable(ip))
            {
                out.push(SocketAddr::new(*ip, socket.port()));
            }
        } else if usable(&socket.ip()) || socket.ip().is_loopback() {
            // A loopback bind is a test setup (`LinkSettings::loopback_only`) — shown as bound.
            out.push(*socket);
        }
    }
    out.sort_by_key(|addr| (addr.is_ipv6(), *addr));
    out.dedup();
    out.into_iter().map(|addr| addr.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_wildcard_bind_lists_every_usable_interface_address_of_its_family() {
        let interfaces = [
            ip("192.168.0.7"),
            ip("10.8.0.2"),
            ip("169.254.10.1"),
            ip("127.0.0.1"),
            ip("fe80::1"),
            ip("fd00::5"),
            ip("2001:db8::7"),
        ];
        let bound: [SocketAddr; 2] = [
            "0.0.0.0:47120".parse().unwrap(),
            "[::]:47120".parse().unwrap(),
        ];
        assert_eq!(
            my_addresses(&bound, &interfaces),
            ["10.8.0.2:47120", "192.168.0.7:47120", "[2001:db8::7]:47120"]
        );
    }

    #[test]
    fn a_specific_bind_is_shown_as_bound_and_nothing_is_invented() {
        let bound: [SocketAddr; 2] = [
            "127.0.0.1:5000".parse().unwrap(),
            "127.0.0.1:5000".parse().unwrap(),
        ];
        assert_eq!(
            my_addresses(&bound, &[ip("192.168.0.7")]),
            ["127.0.0.1:5000"]
        );
        let wildcard: [SocketAddr; 1] = ["0.0.0.0:5000".parse().unwrap()];
        assert!(
            my_addresses(&wildcard, &[]).is_empty(),
            "no interface — no address"
        );
    }

    #[test]
    fn a_read_is_reused_for_the_ttl_and_then_read_again() {
        let cache = Mutex::new(None);
        let t0 = Instant::now();
        assert_eq!(
            cached(&cache, t0, || vec![ip("10.0.0.1")]),
            [ip("10.0.0.1")]
        );
        assert_eq!(
            cached(
                &cache,
                t0 + INTERFACES_TTL - Duration::from_secs(1),
                || panic!("must reuse")
            ),
            [ip("10.0.0.1")]
        );
        assert_eq!(
            cached(&cache, t0 + INTERFACES_TTL, || vec![ip("10.0.0.2")]),
            [ip("10.0.0.2")]
        );
    }

    /// The real call returns without error on this machine (what it finds depends on the machine).
    #[test]
    fn the_interface_query_runs() {
        let found = interface_ips();
        assert!(found.iter().all(|ip| !ip.is_loopback()), "{found:?}");
    }
}

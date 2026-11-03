//! Outbound HTTP that survives a dead address family.
//!
//! ureq 2 tries a host's addresses one after another and spends the whole
//! request timeout on the first. Networks that publish IPv6 DNS records but
//! drop IPv6 traffic resolve `api.anthropic.com` to its IPv6 address first, so
//! every request hangs there and fails while curl and browsers work. They use
//! Happy Eyeballs (RFC 8305); [`agent`] does the same: it races staggered
//! connects across the resolved addresses and hands ureq the first one that
//! answers. The winning probe connection is dropped and ureq reconnects, which
//! costs one extra handshake per request.

use std::{
    collections::VecDeque,
    io,
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

/// RFC 8305's recommended delay before starting the next connection attempt.
const STAGGER: Duration = Duration::from_millis(250);
/// How long the race may take before ureq falls back to the resolver's order.
const RACE: Duration = Duration::from_secs(5);

/// A ureq agent builder whose connections fall back across address families.
pub fn agent() -> ureq::AgentBuilder {
    ureq::AgentBuilder::new().resolver(|netloc: &str| -> io::Result<Vec<SocketAddr>> {
        let addrs = netloc.to_socket_addrs()?.collect();
        Ok(race(
            addrs,
            |addr, timeout| TcpStream::connect_timeout(&addr, timeout).is_ok(),
            STAGGER,
            RACE,
        ))
    })
}

/// Alternates address families, keeping the resolver's preferred family first.
fn interleave(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let preferred_v6 = addrs.first().is_some_and(SocketAddr::is_ipv6);
    let (mut preferred, mut other): (VecDeque<_>, VecDeque<_>) = addrs
        .into_iter()
        .partition(|addr| addr.is_ipv6() == preferred_v6);
    let mut ordered = Vec::with_capacity(preferred.len() + other.len());
    while !preferred.is_empty() || !other.is_empty() {
        ordered.extend(preferred.pop_front());
        ordered.extend(other.pop_front());
    }
    ordered
}

/// Orders `addrs` so the first address that accepts a connection leads. When
/// none does within `limit`, the interleaved order is returned unchanged and
/// ureq reports its own connection error.
fn race<F>(
    addrs: Vec<SocketAddr>,
    connect: F,
    stagger: Duration,
    limit: Duration,
) -> Vec<SocketAddr>
where
    F: Fn(SocketAddr, Duration) -> bool + Send + Sync + 'static,
{
    let mut ordered = interleave(addrs);
    if ordered.len() < 2 {
        return ordered;
    }
    let connect = Arc::new(connect);
    let settled = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    for (attempt, addr) in ordered.iter().copied().enumerate() {
        let delay = stagger.saturating_mul(attempt as u32);
        let Some(budget) = limit.checked_sub(delay).filter(|b| !b.is_zero()) else {
            break;
        };
        let (tx, connect, settled) = (tx.clone(), connect.clone(), settled.clone());
        thread::spawn(move || {
            thread::sleep(delay);
            if !settled.load(Ordering::Relaxed) && connect(addr, budget) {
                let _ = tx.send(addr);
            }
        });
    }
    drop(tx);
    // Ends at the first success, once every attempt has failed, or at `limit`.
    if let Ok(winner) = rx.recv_timeout(limit) {
        settled.store(true, Ordering::Relaxed);
        ordered.retain(|addr| *addr != winner);
        ordered.insert(0, winner);
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, sync::atomic::AtomicUsize, time::Instant};

    const V6: &str = "[2607:6bc0::10]:443";
    const V4: &str = "160.79.104.10:443";
    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn blackholed_ipv6_falls_back_to_ipv4_without_waiting_out_the_hang() {
        // The reported failure: IPv6 resolves first but its connect never answers.
        let started = Instant::now();
        let ordered = race(
            vec![addr(V6), addr(V4)],
            |addr, timeout| {
                if addr.is_ipv6() {
                    thread::sleep(timeout);
                    false
                } else {
                    true
                }
            },
            Duration::from_millis(50),
            Duration::from_secs(5),
        );
        assert_eq!(ordered, vec![addr(V4), addr(V6)]);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "waited {:?} on the dead address",
            started.elapsed()
        );
    }

    #[test]
    fn healthy_preferred_family_keeps_the_lead() {
        let ordered = race(
            vec![addr(V6), addr(V4)],
            |_, _| true,
            Duration::from_millis(100),
            Duration::from_secs(5),
        );
        assert_eq!(ordered, vec![addr(V6), addr(V4)]);
    }

    #[test]
    fn all_unreachable_returns_promptly_in_resolver_order() {
        // ureq still gets every address and reports its own connect error.
        let started = Instant::now();
        let ordered = race(
            vec![addr(V6), addr(V4)],
            |_, _| false,
            Duration::from_millis(10),
            Duration::from_secs(5),
        );
        assert_eq!(ordered, vec![addr(V6), addr(V4)]);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn nothing_answering_gives_up_at_the_limit() {
        let started = Instant::now();
        let ordered = race(
            vec![addr(V6), addr(V4)],
            |_, timeout| {
                thread::sleep(timeout);
                false
            },
            Duration::from_millis(10),
            Duration::from_millis(200),
        );
        assert_eq!(ordered, vec![addr(V6), addr(V4)]);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_single_address_is_not_probed() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let ordered = race(
            vec![addr(V4)],
            move |_, _| {
                seen.fetch_add(1, Ordering::Relaxed);
                true
            },
            STAGGER,
            RACE,
        );
        assert_eq!(ordered, vec![addr(V4)]);
        assert_eq!(calls.load(Ordering::Relaxed), 0, "no extra handshake");
        assert!(race(vec![], |_, _| true, STAGGER, RACE).is_empty());
    }

    #[test]
    fn families_alternate_starting_with_the_resolvers_preference() {
        let (v6b, v4b) = (addr("[2607:6bc0::11]:443"), addr("160.79.104.11:443"));
        assert_eq!(
            interleave(vec![addr(V6), v6b, addr(V4), v4b]),
            vec![addr(V6), addr(V4), v6b, v4b]
        );
        assert_eq!(
            interleave(vec![addr(V4), v4b, addr(V6)]),
            vec![addr(V4), addr(V6), v4b]
        );
    }

    #[test]
    fn real_connects_skip_a_refused_address() {
        let open = TcpListener::bind("127.0.0.1:0").unwrap();
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let open = open.local_addr().unwrap();
        let ordered = race(
            vec![closed, open],
            |addr, timeout| TcpStream::connect_timeout(&addr, timeout).is_ok(),
            Duration::from_millis(50),
            Duration::from_secs(5),
        );
        assert_eq!(ordered[0], open);
    }

    #[test]
    fn agent_serves_requests_after_the_probe_connection() {
        // `localhost` may resolve to ::1 and 127.0.0.1; only IPv4 listens, and
        // the race's own probe connection arrives first and closes unread.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut buf = [0; 1024];
                if stream.read(&mut buf).unwrap_or(0) == 0 {
                    continue;
                }
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .unwrap();
                break;
            }
        });
        let body = agent()
            .timeout(Duration::from_secs(5))
            .build()
            .get(&format!("http://localhost:{port}/"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert_eq!(body, "ok");
        server.join().unwrap();
    }
}

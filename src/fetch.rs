//! The blocking HTTP GET the network panels share.
//!
//! Extracted from four near-identical call sites for one reason: the
//! address-family fallback. On a machine whose resolver returns AAAA records
//! first but which has no IPv6 route — common wherever a network hands out v6
//! addresses it cannot actually route, and confirmed from NetBSD in #205 —
//! every connect dies immediately with `EHOSTUNREACH` before the IPv4 address
//! is ever tried. Browsers and curl mask the same condition with happy-eyeballs
//! fallback, which is exactly why such a machine "has working internet"
//! everywhere except programs that connect in resolver order.
//!
//! `ureq` used to iterate the resolved addresses but move past one only on
//! `ConnectionRefused`, so an unroutable connect bailed with the rest of the
//! list untried. That was fixed upstream in ureq 3.4.2 (ureq#1195), which is
//! this crate's floor, and the fallback stays here anyway: a distribution can
//! build mirador against whatever `ureq` it carries. When a request dies
//! unroutable, it is retried pinned to one address family at a time, which
//! keeps the whole mechanism on `ureq`'s stable config surface rather than
//! reaching into its semver-exempt `unversioned` module.

use std::fmt::Write as _;
use std::io;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use ureq::config::IpFamily;

/// What [`get`] says under `cfg(test)` instead of making a request.
///
/// `quote`'s `the_network_is_refused_while_testing` looks for its last words,
/// `under cfg(test)`, to tell this refusal from a failure on the wire.
const REFUSED: &str = "fetch::get makes no requests under cfg(test)";

/// A blocking GET with a timeout, returning the body as a string.
///
/// The body read is bounded by `ureq`'s 10MB cap, which every caller — news,
/// weather, stocks and the update check — leans on. The agenda reads a local
/// file instead and caps it itself, at the same figure.
///
/// `user_agent` of `None` sends `ureq`'s own default; it is `'static` because
/// it is part of what picks an agent out of [`AGENTS`], and a string written
/// into the source is what keeps that list from growing.
///
/// Each retry gets the full `timeout` again, and that is not the hazard it
/// looks like: the fallback only fires on an *unroutable* connect, and the
/// meaning of unroutable is that the kernel refused at routing level without
/// waiting for anything.
///
/// **Refuses every request under `cfg(test)`**, so "no test touches the
/// network" holds however a panel was built, rather than resting on each
/// caller remembering to refuse for itself.
pub fn get(
    url: &str,
    timeout: Duration,
    user_agent: Option<&'static str>,
) -> Result<String, ureq::Error> {
    if cfg!(test) {
        return Err(ureq::Error::Io(io::Error::other(REFUSED)));
    }
    with_family_fallback(&mut |family| {
        agent_for(user_agent, family)
            .get(url)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .call()?
            .body_mut()
            .read_to_string()
    })
}

/// The agents [`get`] has built, one for each user agent and address family.
///
/// An agent is where `ureq` keeps its connection pool, and its clones share
/// it. `get` used to build a new one for every request, so nothing was ever
/// pooled: the stocks panel's symbols, staggered to one host inside a few
/// seconds, each paid for a fresh connection and TLS handshake.
///
/// Why not one agent for everything, with those two set on each request:
/// `ureq` will not pool a request whose user agent or address family differs
/// from its agent's (`Config::can_share_pool_with` in 3.4.2), and every caller
/// but `news` sends its own user agent. The timeout *is* set per request,
/// because a timeout is one of the things a pooled request may change.
///
/// Bounded by the source rather than by anything read: a key is a `'static`
/// string written into the code and one of three families, so four callers
/// make at most twelve agents. The cost is sockets held open: an idle
/// connection stays in its pool until a later request through that agent
/// finds it past `ureq`'s fifteen-second idle age, and a pool keeps at most
/// three a host and ten in all.
static AGENTS: Mutex<Vec<(AgentKey, ureq::Agent)>> = Mutex::new(Vec::new());

/// What picks an agent out of [`AGENTS`]: the user agent and the family.
type AgentKey = (Option<&'static str>, IpFamily);

/// The agent for these settings, built the first time they are asked for.
fn agent_for(user_agent: Option<&'static str>, family: IpFamily) -> ureq::Agent {
    let key = (user_agent, family);
    // Held for a lookup or a push, never across a request.
    let mut agents = AGENTS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, agent)) = agents.iter().find(|(held, _)| *held == key) {
        return agent.clone();
    }
    let mut config = ureq::Agent::config_builder().ip_family(family);
    if let Some(ua) = user_agent {
        config = config.user_agent(ua);
    }
    let agent = config.build().new_agent();
    agents.push((key, agent.clone()));
    agent
}

/// Percent-encode everything outside RFC 3986's unreserved set.
///
/// The one encoder for a value going into a URL: a stock symbol as a path
/// segment, where `^GSPC` and `EURUSD=X` carry characters that are not safe
/// raw, and a place name as a query value, where `New York` carries a space.
/// Escaping all but the unreserved set is right for both, and it is the
/// allowlist that keeps anything in a symbol from altering the request while
/// `.` passes through for `BRK.B`. Each byte of a multi-byte character is
/// escaped on its own, which is what UTF-8 in a URL means.
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            other => {
                // Writing into a String is infallible.
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

/// Try `IpFamily::Any` first, and on an unroutable connect retry one family
/// at a time.
///
/// Split from [`get`] so the policy can be tested with closures — no test in
/// this repository touches the network.
fn with_family_fallback(
    attempt: &mut dyn FnMut(IpFamily) -> Result<String, ureq::Error>,
) -> Result<String, ureq::Error> {
    let original = match attempt(IpFamily::Any) {
        Err(e) if is_unroutable(&e) => e,
        other => return other,
    };
    for family in [IpFamily::Ipv4Only, IpFamily::Ipv6Only] {
        match attempt(family) {
            // This family resolves to no addresses at all, or dead-ends the
            // same way; the other one may still get through.
            Err(e) if is_unroutable(&e) || matches!(e, ureq::Error::HostNotFound) => {}
            // Anything else reached past routing — a body, or a real answer
            // from a server. A status error is proof the network worked, so
            // it is the answer, not a reason to keep dialling.
            other => return other,
        }
    }
    // Neither family improved on the first attempt. Report the error from
    // the route the system chose, not from a family the host may not have.
    Err(original)
}

/// Whether an error is the kernel refusing a connect at routing level.
///
/// `HostUnreachable` is `EHOSTUNREACH`, the confirmed #205 case;
/// `NetworkUnreachable` is `ENETUNREACH`, which is what Linux returns for the
/// identical v6-without-a-route condition; `AddrNotAvailable` is
/// `EADDRNOTAVAIL`, seen where IPv6 is disabled outright. `ConnectionRefused`
/// is deliberately absent — refused means something answered, and `ureq`
/// already walks the address list for it.
fn is_unroutable(error: &ureq::Error) -> bool {
    let ureq::Error::Io(io) = error else {
        return false;
    };
    matches!(
        io.kind(),
        io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::AddrNotAvailable
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unroutable(kind: io::ErrorKind) -> ureq::Error {
        ureq::Error::Io(io::Error::from(kind))
    }

    /// Both callers' cases, from the two tests that pinned the two private
    /// copies this replaced: `quote` encoded symbols into a path, `weather`
    /// place names into a query. The copies never disagreed — compared on
    /// every Unicode scalar value before they were merged — but the weather
    /// one spelt the space out in an arm of its own, so the space is here by
    /// name; and the sweep holds the whole rule, upper-case hex included, for
    /// every byte a `&str` can carry.
    #[test]
    fn one_encoder_serves_symbols_and_place_names() {
        let cases = [
            ("AAPL", "AAPL"),
            ("^GSPC", "%5EGSPC"),
            ("EURUSD=X", "EURUSD%3DX"),
            ("BRK-B", "BRK-B"),
            ("BRK.B", "BRK.B"),
            ("Boston", "Boston"),
            ("New York", "New%20York"),
            ("a,b", "a%2Cb"),
            ("Zürich", "Z%C3%BCrich"),
            ("a-b_c.d~e", "a-b_c.d~e"),
            ("a/b?c&d#e", "a%2Fb%3Fc%26d%23e"),
        ];
        for (raw, encoded) in cases {
            assert_eq!(percent_encode(raw), encoded, "{raw}");
        }

        let unreserved = |b: u8| b.is_ascii_alphanumeric() || b"-_.~".contains(&b);
        // The Basic Multilingual Plane reaches every byte value a `&str` can
        // hold except the five four-byte leads, `F0` to `F4`, and the last
        // five supply one each.
        let chars = (0..=0xFFFF).chain([0x1_F31E, 0x4_0000, 0x8_0000, 0xC_0000, 0x10_FFFF]);
        for c in chars.filter_map(char::from_u32) {
            let text = c.to_string();
            let expected: String = text
                .bytes()
                .map(|b| {
                    if unreserved(b) {
                        char::from(b).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            assert_eq!(percent_encode(&text), expected, "U+{:04X}", u32::from(c));
        }
    }

    /// One agent for each user agent and family, so a second request with the
    /// same settings reuses the first one's pool rather than building another.
    /// Counted for a user agent no caller sends, so other tests cannot move it.
    #[test]
    fn requests_with_the_same_settings_share_an_agent() {
        const UA: Option<&str> = Some("mirador-test/agent-table");
        let held = || {
            AGENTS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|((ua, _), _)| *ua == UA)
                .count()
        };
        for _ in 0..3 {
            agent_for(UA, IpFamily::Any);
        }
        assert_eq!(
            held(),
            1,
            "three asks with one setting built more than one agent"
        );
        agent_for(UA, IpFamily::Ipv4Only);
        assert_eq!(
            held(),
            2,
            "a family pinned for the fallback is its own agent"
        );
    }

    /// "No test touches the network", held where every request goes through
    /// rather than by each caller. `quote::http_get` refused for itself from
    /// #147 until this guard took its place; weather, news and the update
    /// check call `get` directly, and only `widgets::build` refusing their
    /// panels kept the first two off the wire. A panel built any other way —
    /// `new` rather than `offline` — would have gone out to Open-Meteo. The
    /// address is loopback's discard port, so with the guard deleted this
    /// fails on a refused connect — and sends nothing off the machine unless
    /// a proxy variable routes it through one, since `ureq` reads
    /// `ALL_PROXY` and its siblings and does not exempt loopback.
    #[test]
    fn every_request_is_refused_while_testing() {
        let err = get("http://127.0.0.1:9/", Duration::from_secs(1), None)
            .expect_err("a request must not be possible from a test");
        assert!(
            err.to_string().contains(REFUSED),
            "the refusal must come from the cfg(test) guard, not from a socket: {err}"
        );
    }

    /// #205: DNS answered AAAA-first, the machine had no IPv6 route, and
    /// every fetch died with `No route to host` while curl on the same
    /// machine fell back to the A record and got a 200.
    #[test]
    fn an_unroutable_connect_is_retried_one_family_at_a_time() {
        let mut tried = Vec::new();
        let result = with_family_fallback(&mut |family| {
            tried.push(family);
            match family {
                IpFamily::Ipv4Only => Ok("body".to_string()),
                _ => Err(unroutable(io::ErrorKind::HostUnreachable)),
            }
        });
        assert_eq!(result.unwrap(), "body");
        assert_eq!(tried, vec![IpFamily::Any, IpFamily::Ipv4Only]);
    }

    /// A refused connect means something answered; `ureq` already tries the
    /// rest of the address list for it, and a second agent would not learn
    /// anything the first did not.
    #[test]
    fn a_refused_connect_is_not_retried() {
        let mut attempts = 0;
        let result = with_family_fallback(&mut |_| {
            attempts += 1;
            Err(unroutable(io::ErrorKind::ConnectionRefused))
        });
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }

    /// A status error out of a retry proves the fallback family reached the
    /// server, so it is the real answer — swallowing it in favour of the
    /// original unroutable error would hide a rate limit behind a routing
    /// complaint.
    #[test]
    fn a_status_error_from_a_retry_is_the_answer() {
        let result = with_family_fallback(&mut |family| match family {
            IpFamily::Any => Err(unroutable(io::ErrorKind::HostUnreachable)),
            _ => Err(ureq::Error::StatusCode(429)),
        });
        assert!(matches!(result, Err(ureq::Error::StatusCode(429))));
    }

    /// When no family gets through, the reader sees the error from the route
    /// the system chose — not `HostNotFound` from pinning a family the host
    /// never had addresses for.
    #[test]
    fn a_failed_fallback_reports_the_original_error() {
        let result = with_family_fallback(&mut |family| match family {
            IpFamily::Any => Err(unroutable(io::ErrorKind::HostUnreachable)),
            IpFamily::Ipv4Only => Err(ureq::Error::HostNotFound),
            IpFamily::Ipv6Only => Err(unroutable(io::ErrorKind::NetworkUnreachable)),
        });
        match result {
            Err(ureq::Error::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::HostUnreachable),
            other => panic!("expected the original io error back, got {other:?}"),
        }
    }

    /// The three kinds that mean "refused at routing level", and two that do
    /// not. Losing `NetworkUnreachable` from the list would break the fix on
    /// Linux specifically, which is why each kind is asserted by name.
    #[test]
    fn only_routing_level_failures_trigger_the_fallback() {
        for kind in [
            io::ErrorKind::HostUnreachable,
            io::ErrorKind::NetworkUnreachable,
            io::ErrorKind::AddrNotAvailable,
        ] {
            assert!(is_unroutable(&unroutable(kind)), "{kind:?} should trigger");
        }
        for kind in [io::ErrorKind::ConnectionRefused, io::ErrorKind::TimedOut] {
            assert!(!is_unroutable(&unroutable(kind)), "{kind:?} should not");
        }
        assert!(!is_unroutable(&ureq::Error::StatusCode(500)));
    }
}

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
//! `ureq` iterates the resolved addresses but moves past one only on
//! `ConnectionRefused`; an unroutable connect makes it bail with the rest of
//! the list untried. Until that is fixed upstream, the fallback lives here:
//! when a request dies unroutable, it is retried pinned to one address family
//! at a time, which keeps the whole mechanism on `ureq`'s stable config
//! surface rather than reaching into its semver-exempt `unversioned` module.

use std::io;
use std::time::Duration;

use ureq::config::IpFamily;

/// A blocking GET with a timeout, returning the body as a string.
///
/// The body read is bounded by `ureq`'s 10MB cap, which `feed` and `agenda`
/// both lean on. `user_agent` of `None` sends `ureq`'s own default.
///
/// Each retry gets the full `timeout` again, and that is not the hazard it
/// looks like: the fallback only fires on an *unroutable* connect, and the
/// meaning of unroutable is that the kernel refused at routing level without
/// waiting for anything.
pub fn get(url: &str, timeout: Duration, user_agent: Option<&str>) -> Result<String, ureq::Error> {
    with_family_fallback(&mut |family| {
        let mut config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .ip_family(family);
        if let Some(ua) = user_agent {
            config = config.user_agent(ua);
        }
        let agent = config.build().new_agent();
        agent.get(url).call()?.body_mut().read_to_string()
    })
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

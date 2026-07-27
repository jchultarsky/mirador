//! Telling you a newer version exists, if — and only if — you asked to be told.
//!
//! # Off by default, and that is not laziness
//!
//! The README promises that mirador does not phone home. An update check is a
//! network request that reveals your IP address and the fact that you run this
//! program, to a third party, on a schedule you did not choose. That is a small
//! thing and it is still the thing the promise was about, so it is opt-in:
//! `[general].check_for_updates = true` and nothing else turns it on.
//!
//! Everything below follows from taking that seriously.
//!
//! - **`NO_UPDATE_CHECK` and `DO_NOT_TRACK` win over the config.** Someone who
//!   sets those in their environment has said what they want more recently than
//!   whoever edited the config, and on a shared or managed machine they may not
//!   be the same person.
//! - **crates.io, not GitHub.** It is where `cargo install mirador` gets its
//!   answer, it needs no token, and its unauthenticated rate limit is not a
//!   trap the way GitHub's 60-per-hour-per-IP is behind a shared NAT.
//! - **Nothing is sent but the request.** No identifier, no configuration, no
//!   count of anything. The reply is one version string.
//! - **The result is cached to disk**, so leaving the dashboard open all day and
//!   restarting it twenty times is one request, not twenty.
//! - **A failure is silent.** No network, a proxy, an outage, a change to the
//!   API — none of that is the user's problem, and a dashboard that complains
//!   about its own update check has become the thing it was meant to save you
//!   from.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// How long a check is good for.
///
/// A day. Releases are not frequent enough for anything shorter to tell you
/// something new, and the point of the cache is that an eager restarter and a
/// week-long session cost the same.
const CACHE_FOR: Duration = Duration::from_hours(24);

/// Short, because nobody is waiting for this and a hung request should not keep
/// a thread alive for long after the dashboard has quit.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

const ENDPOINT: &str = "https://crates.io/api/v1/crates/mirador";

/// What the last check found, persisted between runs.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Cache {
    /// The newest stable version crates.io reported.
    latest: Option<String>,
    /// When that was, as seconds since the Unix epoch.
    ///
    /// Wall-clock rather than `Instant`, because the whole point is to survive
    /// a restart, and an `Instant` means nothing across one. A clock moved
    /// backwards makes this look stale and costs one extra request.
    checked_at: Option<i64>,
}

/// A newer version, once something has found one.
pub type Found = Arc<Mutex<Option<String>>>;

/// Start a check on a background thread, if one is wanted and one is due.
///
/// Returns the slot the answer will appear in. The slot stays empty when
/// checking is off, when the cache is fresh and had nothing to report, or when
/// anything at all goes wrong.
pub fn spawn(enabled: bool, path: Option<PathBuf>) -> Found {
    let found: Found = Arc::new(Mutex::new(None));

    if !enabled || opted_out() {
        return found;
    }
    let Some(path) = path else {
        return found;
    };

    // A cached answer is used immediately, so a newer version found yesterday
    // is on screen this morning without waiting for a request to come back.
    let cache = read_cache(&path);
    if let Some(latest) = cache.latest.as_deref()
        && is_newer(latest, current())
    {
        set(&found, Some(latest.to_string()));
    }
    if fresh(&cache) {
        return found;
    }

    let shared = Arc::clone(&found);
    // Failing to spawn is not worth taking the dashboard down for; without a
    // thread there is simply no check.
    let _ = std::thread::Builder::new()
        .name("mirador-update".into())
        .spawn(move || {
            let Ok(latest) = fetch(ENDPOINT) else {
                return;
            };
            // Written even when it matches what is installed: the point of the
            // cache is to stop asking, and "nothing new" is an answer.
            let _ = write_cache(
                &path,
                &Cache {
                    latest: Some(latest.clone()),
                    checked_at: Some(now_seconds()),
                },
            );
            if is_newer(&latest, current()) {
                set(&shared, Some(latest));
            }
        });

    found
}

/// Whether the environment forbids this regardless of the config.
///
/// `DO_NOT_TRACK` is the cross-tool convention; `NO_UPDATE_CHECK` is the
/// specific one. Either being set to anything other than `0` is a no.
fn opted_out() -> bool {
    opted_out_given(|key| std::env::var(key).ok())
}

/// The decision, with the environment as a parameter.
///
/// Split out because `std::env::set_var` is `unsafe` in the 2024 edition and
/// this crate forbids `unsafe` outright — but also because a test that mutates
/// process-global state leaks into every test that runs after it.
fn opted_out_given(read: impl Fn(&str) -> Option<String>) -> bool {
    ["NO_UPDATE_CHECK", "DO_NOT_TRACK"]
        .iter()
        .filter_map(|key| read(key))
        .any(|value| !value.is_empty() && value != "0")
}

fn set(slot: &Found, value: Option<String>) {
    match slot.lock() {
        Ok(mut guard) => *guard = value,
        Err(poisoned) => *poisoned.into_inner() = value,
    }
}

/// The version this binary was built as.
fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn now_seconds() -> i64 {
    jiff::Timestamp::now().as_second()
}

fn fresh(cache: &Cache) -> bool {
    let Some(checked_at) = cache.checked_at else {
        return false;
    };
    let age = now_seconds().saturating_sub(checked_at);
    // A negative age means the clock moved; treat it as stale rather than as
    // valid until the far future.
    (0..CACHE_FOR.as_secs().cast_signed()).contains(&age)
}

fn read_cache(path: &Path) -> Cache {
    // Every failure is the same failure: there is no usable cache, so check.
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_cache(path: &Path, cache: &Cache) -> Result<()> {
    let body = toml::to_string_pretty(cache)?;
    crate::store::write_atomic(
        path,
        &format!(
            "# Written by mirador's update check, which is off unless\n\
             # `[general].check_for_updates` turns it on. Safe to delete.\n\n{body}"
        ),
    )
}

/// Ask crates.io for the newest stable version.
fn fetch(url: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Body {
        #[serde(rename = "crate")]
        krate: Krate,
    }
    #[derive(Deserialize)]
    struct Krate {
        max_stable_version: Option<String>,
        max_version: Option<String>,
    }

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        // crates.io asks for a user agent that says who is calling and how to
        // get in touch. Honouring that is the price of using the API politely.
        .user_agent(concat!(
            "mirador/",
            env!("CARGO_PKG_VERSION"),
            " (update check; https://github.com/jchultarsky/mirador)"
        ))
        .build()
        .new_agent();

    // Read as text then parse, matching how the other two network paths in this
    // crate do it — it keeps the JSON handling in one crate rather than two.
    let text = agent.get(url).call()?.body_mut().read_to_string()?;
    let body: Body = serde_json::from_str(&text)?;
    // `max_stable_version` skips pre-releases, which is what someone running a
    // release build wants to hear about. It is absent on a crate that has only
    // ever published a pre-release, hence the fallback.
    body.krate
        .max_stable_version
        .or(body.krate.max_version)
        .ok_or_else(|| anyhow::anyhow!("no version in the response"))
}

/// Whether `candidate` is a later release than `installed`.
///
/// Compares `major.minor.patch` numerically. Anything carrying a pre-release or
/// build suffix is treated as **not** newer: mirador only ever publishes plain
/// releases, `max_stable_version` excludes pre-releases anyway, and refusing to
/// guess is better than telling someone `0.8.0-rc.1` supersedes `0.8.0`.
///
/// This is deliberately not the `semver` crate. It is one comparison of three
/// integers, and the dependency would be the fourth-largest thing in the tree
/// for it.
fn is_newer(candidate: &str, installed: &str) -> bool {
    let (Some(candidate), Some(installed)) = (triple(candidate), triple(installed)) else {
        return false;
    };
    candidate > installed
}

fn triple(version: &str) -> Option<(u64, u64, u64)> {
    let version = version.trim();
    if version.contains(['-', '+']) {
        return None;
    }
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    // A fourth component is not a version this understands.
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Where the cache lives, beside the other files mirador owns.
pub fn default_path(data_dir: &Path) -> PathBuf {
    data_dir.join("update-check.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_version_in_any_position_is_newer() {
        assert!(is_newer("0.7.2", "0.7.1"));
        assert!(is_newer("0.8.0", "0.7.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.10.0", "0.9.0"), "10 is not less than 9");
    }

    #[test]
    fn the_same_or_an_older_version_is_not_newer() {
        assert!(!is_newer("0.7.1", "0.7.1"));
        assert!(!is_newer("0.7.0", "0.7.1"));
        assert!(!is_newer("0.6.9", "0.7.0"));
    }

    #[test]
    fn a_prerelease_never_counts_as_newer() {
        // Telling someone that 0.8.0-rc.1 supersedes their 0.8.0 would be
        // wrong, and telling them it supersedes 0.7.0 is a judgement this does
        // not have the information to make.
        assert!(!is_newer("0.8.0-rc.1", "0.7.0"));
        assert!(!is_newer("0.8.0-rc.1", "0.8.0"));
        assert!(!is_newer("0.8.0+build.5", "0.7.0"));
    }

    #[test]
    fn an_unparseable_version_is_never_newer() {
        // A changed API, a truncated reply, a proxy serving an error page. None
        // of them should put a notice on screen.
        for junk in ["", "latest", "0.7", "0.7.1.2", "v0.7.2", "not a version"] {
            assert!(!is_newer(junk, "0.7.1"), "{junk:?} was treated as newer");
        }
        assert!(!is_newer("0.7.2", "garbage"));
    }

    #[test]
    fn a_cache_is_fresh_for_a_day_and_then_is_not() {
        let at = |seconds_ago: i64| Cache {
            latest: Some("0.7.1".into()),
            checked_at: Some(now_seconds() - seconds_ago),
        };
        assert!(fresh(&at(0)));
        assert!(fresh(&at(60 * 60 * 23)));
        assert!(!fresh(&at(60 * 60 * 25)));
        assert!(!fresh(&Cache::default()), "never checked is not fresh");
    }

    #[test]
    fn a_clock_that_moved_backwards_makes_the_cache_stale_rather_than_eternal() {
        // Stamped in the future: without the lower bound this reads as fresh
        // until the clock catches up, which could be indefinitely.
        let future = Cache {
            latest: Some("0.7.1".into()),
            checked_at: Some(now_seconds() + 60 * 60 * 24 * 365),
        };
        assert!(!fresh(&future));
    }

    #[test]
    fn the_environment_can_veto_the_config() {
        let env = |set: &'static str, value: &'static str| {
            move |key: &str| (key == set).then(|| value.to_string())
        };

        for key in ["NO_UPDATE_CHECK", "DO_NOT_TRACK"] {
            assert!(opted_out_given(env(key, "1")), "{key} was ignored");
            assert!(opted_out_given(env(key, "true")), "{key}=true was ignored");
            // `0` and empty are the conventional ways to say "not set" without
            // unsetting; honouring them keeps a shell profile from opting
            // someone out permanently by accident.
            assert!(
                !opted_out_given(env(key, "0")),
                "{key}=0 should not opt out"
            );
            assert!(!opted_out_given(env(key, "")), "{key}= should not opt out");
        }
        assert!(!opted_out_given(|_| None), "nothing set is not opted out");
    }

    #[test]
    fn nothing_is_spawned_when_the_check_is_off() {
        // The property that keeps the README's promise true: with the setting
        // off there is no thread, no request, and nothing to report.
        let found = spawn(false, Some(PathBuf::from("/nonexistent/update-check.toml")));
        assert!(found.lock().unwrap().is_none());
    }

    #[test]
    fn an_unreadable_cache_is_treated_as_no_cache() {
        assert!(
            read_cache(Path::new("/nonexistent/nope.toml"))
                .checked_at
                .is_none()
        );
    }

    #[test]
    fn a_cache_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("mirador-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = default_path(&dir);

        let written = Cache {
            latest: Some("9.9.9".into()),
            checked_at: Some(now_seconds()),
        };
        write_cache(&path, &written).unwrap();
        let read = read_cache(&path);
        assert_eq!(read.latest.as_deref(), Some("9.9.9"));
        assert!(fresh(&read));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

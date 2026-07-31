//! Share prices: the data model, the pluggable source behind it, and the
//! persisted watchlist.
//!
//! **The source is a trait on purpose, from the first commit.** Yahoo gates on
//! IP reputation rather than on authentication, and blocks datacenter and VPN
//! ranges wholesale — so the default source works perfectly on a laptop and
//! fails flatly on a VPS, for the same user with the same build. That is not a
//! bug anyone can fix in the client; it can only be routed around by using a
//! different source. Making that swappable later would mean rewriting the panel.
//!
//! Licensing ruled out several otherwise-obvious providers, and the reasoning
//! is recorded here so nobody re-derives it:
//!
//! - **Finnhub is unusable at any tier.** Its terms restrict *every* plan,
//!   including paid ones, to personal use, so no user of a distributed tool can
//!   ever be compliant.
//! - **FMP** forbids displaying its data in a software product.
//! - **Alpha Vantage** allows 25 requests a day, which is not a watchlist.
//!
//! Operational rules, also deliberate: never bundle a shared API key (it would
//! be a shared secret in a public binary, and rate limits are per key); poll no
//! faster than once a minute; request symbols one at a time with a pause
//! between them rather than firing a burst; and never write quotes to disk —
//! prices are session state, and a stale price read as live is worse than no
//! price.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How long any single request may take.
///
/// `cfg(not(test))` alongside the only function that uses it: under `cfg(test)`
/// `http_get` refuses before building an agent, so keeping these would be dead
/// code the lint would rightly complain about.
#[cfg(not(test))]
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Yahoo rejects requests without a browser user agent with HTTP 429, whatever
/// the request rate. This is not an attempt to hide what mirador is — the
/// endpoint simply refuses anything that does not look like a browser.
#[cfg(not(test))]
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                          AppleWebKit/537.36 (KHTML, like Gecko) \
                          Chrome/126.0.0.0 Safari/537.36";

/// One symbol's current state.
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    /// Yesterday's close, which is what the day's change is measured against.
    pub previous_close: f64,
    pub currency: Option<String>,
    /// Intraday closes, oldest first, for the sparkline. May be empty.
    pub series: Vec<f64>,
    /// Whether the source declares its data delayed.
    pub delayed: bool,
}

impl Quote {
    pub fn change(&self) -> f64 {
        self.price - self.previous_close
    }

    /// Day change as a percentage.
    ///
    /// Returns zero rather than infinity when the previous close is zero, which
    /// happens for a symbol that has never traded. A row reading `inf%` looks
    /// like a broken panel; `0.00%` reads as "nothing has happened".
    pub fn change_pct(&self) -> f64 {
        if self.previous_close.abs() < f64::EPSILON {
            return 0.0;
        }
        (self.change() / self.previous_close) * 100.0
    }
}

/// Where quotes come from.
///
/// Object-safe, because the panel holds one as a `Box<dyn QuoteSource>` chosen
/// by config at startup.
pub trait QuoteSource: Send {
    /// Name shown in the panel, so a user can tell which source produced a
    /// number — and which one is failing.
    fn name(&self) -> &'static str;

    /// Fetch one symbol. Implementations must not retry internally; the panel
    /// owns the cadence.
    fn fetch(&self, symbol: &str) -> Result<Quote>;
}

/// Every source id accepted in `[stocks].source`.
pub const SOURCE_NAMES: &[&str] = &["yahoo"];

/// Build a source by id.
pub fn source_for(name: &str) -> Option<Box<dyn QuoteSource>> {
    match name {
        "yahoo" => Some(Box::new(YahooChart)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Yahoo
// ---------------------------------------------------------------------------

/// Yahoo Finance's `/v8/finance/chart/` endpoint.
///
/// Chosen over `/v7/finance/quote`, which now needs a cookie-and-crumb pair and
/// returns 401 without one. One GET to v8 returns the price, the previous
/// close, the currency *and* the intraday series, so a watchlist row costs
/// exactly one request.
pub struct YahooChart;

#[derive(Debug, Deserialize)]
struct ChartEnvelope {
    chart: ChartBody,
}

#[derive(Debug, Deserialize)]
struct ChartBody {
    #[serde(default)]
    result: Vec<ChartResult>,
    #[serde(default)]
    error: Option<ChartError>,
}

#[derive(Debug, Deserialize)]
struct ChartError {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChartResult {
    meta: ChartMeta,
    #[serde(default)]
    indicators: Option<Indicators>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartMeta {
    symbol: String,
    #[serde(default)]
    regular_market_price: Option<f64>,
    #[serde(default)]
    chart_previous_close: Option<f64>,
    #[serde(default)]
    previous_close: Option<f64>,
    #[serde(default)]
    currency: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Indicators {
    #[serde(default)]
    quote: Vec<QuoteSeries>,
}

#[derive(Debug, Deserialize)]
struct QuoteSeries {
    /// Nulls appear for minutes with no trades, so the element type is optional
    /// and the gaps are dropped rather than read as zero — a zero would draw
    /// the sparkline straight to the floor.
    #[serde(default)]
    close: Vec<Option<f64>>,
}

/// Turn a v8 chart response into a [`Quote`].
///
/// Split from the request so it can be tested against captured JSON; the test
/// suite makes no network calls.
pub fn parse_chart(body: &str) -> Result<Quote> {
    let parsed: ChartEnvelope = serde_json::from_str(body).context("parsing the chart response")?;

    if let Some(error) = parsed.chart.error {
        let detail = error
            .description
            .or(error.code)
            .unwrap_or_else(|| "unknown error".to_string());
        anyhow::bail!("the quote service rejected the symbol: {detail}");
    }

    let result = parsed
        .chart
        .result
        .into_iter()
        .next()
        .context("the quote service returned no data for this symbol")?;

    let price = result
        .meta
        .regular_market_price
        .context("the response carried no price")?;

    // `chartPreviousClose` is the one that lines up with the returned series;
    // `previousClose` is the fallback for responses that omit it.
    let previous_close = result
        .meta
        .chart_previous_close
        .or(result.meta.previous_close)
        .unwrap_or(price);

    let series = result
        .indicators
        .and_then(|i| i.quote.into_iter().next())
        .map(|q| q.close.into_iter().flatten().collect())
        .unwrap_or_default();

    Ok(Quote {
        symbol: result.meta.symbol,
        price,
        previous_close,
        currency: result.meta.currency,
        series,
        delayed: false,
    })
}

impl QuoteSource for YahooChart {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    fn fetch(&self, symbol: &str) -> Result<Quote> {
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}\
             ?range=1d&interval=5m",
            urlencode(symbol)
        );
        let body = http_get(&url)?;
        parse_chart(&body)
    }
}

/// A blocking GET with a timeout, presenting a browser user agent.
///
/// **Refuses outright under `cfg(test)`, and that is load-bearing (#147).**
/// This is the only function in the crate that opens a socket, and the stated
/// rule is that no test does. The rule was believed to hold because
/// `parse_chart` is tested against captured JSON — but a panel spawns a fetch
/// thread when it is *constructed*, and `Layout::default()` places the stocks
/// panel, so building a dashboard in a test was enough. A Windows runner got a
/// real HTTP 404 back for a made-up symbol and rendered it.
///
/// Injecting a `QuoteSource` fixes the tests that build a panel directly, and
/// is the better design; this covers the ones that reach it through
/// `widgets::build`, where there is no seam to pass a source through. Nothing
/// is lost: no test asserts on a live response, because none could.
#[cfg(test)]
fn http_get(_url: &str) -> Result<String> {
    anyhow::bail!("network access is refused in tests")
}

#[cfg(not(test))]
fn http_get(url: &str) -> Result<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .user_agent(BROWSER_UA)
        .build()
        .new_agent();

    let mut response = agent.get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(429) => anyhow::anyhow!(
            "the quote service is rate-limiting this network (HTTP 429). \
             Yahoo blocks datacenter and VPN addresses outright; on such a \
             connection no polling rate will help and another `[stocks].source` \
             is needed."
        ),
        ureq::Error::StatusCode(404) => {
            anyhow::anyhow!("no such symbol")
        }
        ureq::Error::StatusCode(code) => {
            anyhow::anyhow!("the quote service returned HTTP {code}")
        }
        other => anyhow::anyhow!("network request failed: {other}"),
    })?;

    response
        .body_mut()
        .read_to_string()
        .context("reading the response body")
}

/// Percent-encode the characters that matter in a path segment.
///
/// Symbols carry `^` for indices (`^GSPC`) and `=` for futures and currencies
/// (`EURUSD=X`), neither of which is safe raw.
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => {
                // Writing into a String is infallible.
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The watchlist
// ---------------------------------------------------------------------------

/// Symbols the user is watching, persisted so the UI can add and remove them.
///
/// This is a *data* file next to the tasks and notes, not part of the config.
/// That is the whole reason the panel can edit it: mirador deliberately never
/// rewrites the config file, so a watchlist that lived there could only ever be
/// changed in an editor. Config still seeds the first run and nothing more.
#[derive(Debug)]
pub struct Watchlist {
    path: PathBuf,
    symbols: Vec<String>,
    dirty: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WatchlistFile {
    #[serde(default)]
    symbols: Vec<String>,
}

impl Watchlist {
    /// Load from `path`, falling back to `seed` when the file does not exist.
    pub fn load(path: impl Into<PathBuf>, seed: &[String]) -> Result<Self> {
        let path = path.into();
        let (symbols, dirty) = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading the watchlist from {}", path.display()))?;
            let parsed: WatchlistFile = toml::from_str(&raw)
                .with_context(|| format!("parsing the watchlist in {}", path.display()))?;
            (parsed.symbols, false)
        } else {
            // Seeded, and marked dirty so the seed is written out once and the
            // user has a file to edit by hand if they prefer.
            (seed.to_vec(), !seed.is_empty())
        };

        Ok(Self {
            path,
            symbols: symbols.into_iter().map(|s| normalise(&s)).collect(),
            dirty,
            last_error: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// Add a symbol. Returns false when it is blank or already present.
    pub fn add(&mut self, symbol: &str) -> bool {
        let symbol = normalise(symbol);
        if symbol.is_empty() || self.symbols.contains(&symbol) {
            return false;
        }
        self.symbols.push(symbol);
        self.dirty = true;
        true
    }

    pub fn remove(&mut self, symbol: &str) -> bool {
        let symbol = normalise(symbol);
        let before = self.symbols.len();
        self.symbols.retain(|s| *s != symbol);
        let removed = self.symbols.len() != before;
        self.dirty |= removed;
        removed
    }

    /// Write atomically if there are pending changes.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let file = WatchlistFile {
            symbols: self.symbols.clone(),
        };
        let body = toml::to_string_pretty(&file).context("serialising the watchlist")?;
        let contents = format!(
            "# mirador watchlist. Safe to edit by hand.\n\
             # Only the symbols live here — prices are never written to disk.\n\n{body}"
        );

        crate::store::write_atomic(&self.path, &contents)?;

        self.dirty = false;
        Ok(())
    }

    pub fn save_reporting(&mut self) {
        crate::store::report(self.save(), &mut self.last_error);
    }
}

/// Symbols are upper-case and unpadded everywhere, so `aapl` and `AAPL ` cannot
/// both end up in the list as separate entries.
fn normalise(symbol: &str) -> String {
    symbol.trim().to_uppercase()
}

// ---------------------------------------------------------------------------
// Sparkline
// ---------------------------------------------------------------------------

/// Eight levels in a single cell.
const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A one-row sparkline of `series`, exactly `width` cells wide.
///
/// Blocks rather than the braille used by the CPU and network graphs: braille
/// packs two samples into a cell by splitting it vertically, which is the right
/// trade when a graph owns several rows and needs horizontal resolution. In a
/// single row inside a table it just halves the height resolution for detail
/// nobody can read at that size.
///
/// A flat series draws at mid-height rather than at the floor, because a stock
/// that has not moved has not crashed.
pub fn sparkline(series: &[f64], width: usize) -> String {
    if width == 0 || series.is_empty() {
        return String::new();
    }

    let low = series.iter().copied().fold(f64::INFINITY, f64::min);
    let high = series.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = high - low;

    (0..width)
        .map(|cell| {
            // Bucket the samples so the whole series is represented however
            // narrow the column is, rather than showing only the first `width`
            // of them.
            let start = cell * series.len() / width;
            let end = (((cell + 1) * series.len()) / width).max(start + 1);
            let bucket = &series[start.min(series.len() - 1)..end.min(series.len())];
            if bucket.is_empty() {
                return BLOCKS[0];
            }
            let mean = bucket.iter().sum::<f64>() / bucket.len() as f64;

            if span.abs() < f64::EPSILON {
                return BLOCKS[BLOCKS.len() / 2];
            }
            let ratio = (mean - low) / span;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "ratio is clamped to 0..=1, so the product is a valid index"
            )]
            let index = (ratio * (BLOCKS.len() - 1) as f64).round() as usize;
            BLOCKS[index.min(BLOCKS.len() - 1)]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule that no test opens a socket, made checkable (#147).
    ///
    /// It used to rest on `parse_chart` being split from the request — true,
    /// and not enough: constructing a stocks panel spawns a fetch thread, and
    /// `Layout::default()` places that panel, so building a dashboard in a test
    /// was sufficient to call Yahoo. `http_get` refuses under `cfg(test)` now.
    /// Delete that branch and this goes red rather than going quiet.
    #[test]
    fn the_network_is_refused_while_testing() {
        let err = YahooChart
            .fetch("AAPL")
            .expect_err("a live fetch must not be possible from a test");
        assert!(
            err.to_string().contains("refused in tests"),
            "the refusal must come from the cfg(test) guard, not from the wire: {err}"
        );
    }

    /// A trimmed but structurally faithful v8 chart response.
    const SAMPLE: &str = r#"{
      "chart": {
        "result": [{
          "meta": {
            "currency": "USD",
            "symbol": "AAPL",
            "regularMarketPrice": 213.5,
            "chartPreviousClose": 211.0,
            "previousClose": 210.0
          },
          "timestamp": [1, 2, 3, 4],
          "indicators": {
            "quote": [{ "close": [211.0, null, 212.5, 213.5] }]
          }
        }],
        "error": null
      }
    }"#;

    /// A quote response is untrusted network input carrying numbers straight
    /// into arithmetic and onto a screen. These are the shapes that break float
    /// code: a value too large for an `f64`, a divisor of zero, a series with
    /// no span, nothing at all.
    ///
    /// The outcome to insist on is that a row never reads `inf%` or `NaN`. A
    /// wrong-looking number in a money column is worse than a missing one,
    /// which is the same reasoning that keeps stale prices off this panel.
    #[test]
    fn hostile_numbers_never_reach_the_screen_as_inf_or_nan() {
        let swap = |price: &str, prev: &str, series: &str| {
            SAMPLE
                .replace("213.5,", &format!("{price},"))
                .replace(
                    "\"chartPreviousClose\": 211.0",
                    &format!("\"chartPreviousClose\": {prev}"),
                )
                .replace("[211.0, null, 212.5, 213.5]", series)
        };

        for (name, price, prev, series) in [
            (
                "a price too large for an f64",
                "1e400",
                "211.0",
                "[211.0, 212.5]",
            ),
            (
                "a previous close too large",
                "213.5",
                "1e400",
                "[211.0, 212.5]",
            ),
            (
                "a value in the series too large",
                "1.0",
                "1.0",
                "[1e400, 1.0]",
            ),
            ("a previous close of zero", "213.5", "0.0", "[211.0, 212.5]"),
            ("negative prices", "-5.0", "-10.0", "[-1.0, -2.0, -3.0]"),
            ("an empty series", "1.0", "1.0", "[]"),
            ("a flat series", "1.0", "1.0", "[7.0, 7.0, 7.0]"),
        ] {
            // Out-of-range literals are refused outright by the JSON parser,
            // which is a stronger defence than any guard downstream — and one
            // worth pinning, since it is the reason no guard is needed for them.
            let Ok(quote) = parse_chart(&swap(price, prev, series)) else {
                continue;
            };

            assert!(quote.price.is_finite(), "{name}: price is {}", quote.price);
            assert!(
                quote.change_pct().is_finite(),
                "{name}: `{}%` would read as a broken panel",
                quote.change_pct()
            );
            // And the sparkline neither panics nor overruns its width.
            for width in [0, 1, 8, 40] {
                assert!(
                    sparkline(&quote.series, width).chars().count() <= width,
                    "{name}: sparkline overran {width} cells"
                );
            }
        }
    }

    #[test]
    fn a_chart_response_becomes_a_quote() {
        let q = parse_chart(SAMPLE).unwrap();
        assert_eq!(q.symbol, "AAPL");
        assert!((q.price - 213.5).abs() < f64::EPSILON);
        assert!(
            (q.previous_close - 211.0).abs() < f64::EPSILON,
            "chartPreviousClose wins: it matches the returned series"
        );
        assert_eq!(q.currency.as_deref(), Some("USD"));
    }

    #[test]
    fn gaps_in_the_series_are_dropped_rather_than_read_as_zero() {
        let q = parse_chart(SAMPLE).unwrap();
        assert_eq!(
            q.series,
            vec![211.0, 212.5, 213.5],
            "a null minute must not become a zero and floor the sparkline"
        );
    }

    #[test]
    fn the_day_change_is_measured_against_the_previous_close() {
        let q = parse_chart(SAMPLE).unwrap();
        assert!((q.change() - 2.5).abs() < 1e-9);
        assert!((q.change_pct() - (2.5 / 211.0 * 100.0)).abs() < 1e-9);
    }

    #[test]
    fn a_zero_previous_close_reports_no_change_rather_than_infinity() {
        let q = Quote {
            symbol: "NEW".into(),
            price: 10.0,
            previous_close: 0.0,
            currency: None,
            series: vec![],
            delayed: false,
        };
        assert!(q.change_pct().is_finite(), "`inf%` reads as a broken panel");
        assert!((q.change_pct() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn an_error_envelope_becomes_an_error_not_an_empty_quote() {
        let body = r#"{"chart":{"result":[],"error":{"code":"Not Found",
                       "description":"No data found, symbol may be delisted"}}}"#;
        let err = parse_chart(body).unwrap_err().to_string();
        assert!(err.contains("delisted"), "got `{err}`");
    }

    #[test]
    fn a_response_with_no_price_is_an_error_rather_than_a_zero() {
        let body = r#"{"chart":{"result":[{"meta":{"symbol":"X"}}],"error":null}}"#;
        assert!(
            parse_chart(body).is_err(),
            "a missing price must not render as 0.00"
        );
    }

    #[test]
    fn a_response_without_a_series_still_yields_a_quote() {
        // The panel must survive a source that has no intraday data: the price
        // is the point, the sparkline is a bonus.
        let body = r#"{"chart":{"result":[{"meta":{"symbol":"X",
                       "regularMarketPrice":5.0,"previousClose":4.0}}],"error":null}}"#;
        let q = parse_chart(body).unwrap();
        assert!(q.series.is_empty());
        assert!((q.change() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn symbols_are_encoded_so_indices_and_currencies_survive_the_url() {
        assert_eq!(urlencode("AAPL"), "AAPL");
        assert_eq!(urlencode("^GSPC"), "%5EGSPC", "an index carries a caret");
        assert_eq!(urlencode("EURUSD=X"), "EURUSD%3DX", "a pair carries an =");
        assert_eq!(urlencode("BRK-B"), "BRK-B", "a hyphen is already safe");
    }

    #[test]
    fn only_advertised_sources_can_be_built() {
        for name in SOURCE_NAMES {
            assert!(
                source_for(name).is_some(),
                "{name} is advertised but absent"
            );
        }
        assert!(source_for("finnhub").is_none(), "ruled out on licence");
        assert!(source_for("").is_none());
    }

    // -- sparkline ----------------------------------------------------------

    #[test]
    fn a_sparkline_is_exactly_as_wide_as_it_was_asked_to_be() {
        for width in [1, 5, 12, 40] {
            let line = sparkline(&[1.0, 2.0, 3.0, 4.0, 5.0], width);
            assert_eq!(line.chars().count(), width, "at width {width}");
        }
    }

    #[test]
    fn a_rising_series_ends_higher_than_it_starts() {
        let line: Vec<char> = sparkline(&[1.0, 2.0, 3.0, 4.0], 4).chars().collect();
        assert_eq!(line[0], BLOCKS[0]);
        assert_eq!(line[3], BLOCKS[7]);
    }

    #[test]
    fn a_flat_series_draws_at_mid_height_not_on_the_floor() {
        let line = sparkline(&[7.0; 6], 6);
        assert!(
            line.chars().all(|c| c == BLOCKS[BLOCKS.len() / 2]),
            "a stock that has not moved has not crashed: `{line}`"
        );
    }

    #[test]
    fn a_sparkline_covers_the_whole_series_even_when_narrow() {
        // Two cells for ten samples must average across all ten. Taking only
        // the first two would hide the whole day and leave the two cells
        // almost level; averaging puts them far apart.
        let level = |c: char| BLOCKS.iter().position(|b| *b == c).unwrap();

        let mut series: Vec<f64> = (0..10).map(f64::from).collect();
        let line: Vec<char> = sparkline(&series, 2).chars().collect();
        assert!(
            level(line[1]) >= level(line[0]) + 3,
            "the tail of a rising series must sit well above its head: `{}`",
            line.iter().collect::<String>()
        );

        series.reverse();
        let line: Vec<char> = sparkline(&series, 2).chars().collect();
        assert!(
            level(line[0]) >= level(line[1]) + 3,
            "and a falling series inverts: `{}`",
            line.iter().collect::<String>()
        );
    }

    #[test]
    fn degenerate_sparklines_do_not_panic() {
        assert_eq!(sparkline(&[], 10), "");
        assert_eq!(sparkline(&[1.0], 0), "");
        assert_eq!(sparkline(&[1.0], 3).chars().count(), 3);
    }

    // -- watchlist ----------------------------------------------------------

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn watchlist(name: &str, seed: &[&str]) -> (Watchlist, TempDir) {
        let dir = std::env::temp_dir().join(format!("mirador-watch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let seed: Vec<String> = seed.iter().map(|s| (*s).to_string()).collect();
        let list = Watchlist::load(dir.join("watchlist.toml"), &seed).unwrap();
        (list, TempDir(dir))
    }

    #[test]
    fn a_missing_watchlist_is_seeded_from_config_and_then_owns_itself() {
        let (mut list, guard) = watchlist("seed", &["AAPL", "MSFT"]);
        assert_eq!(list.symbols(), ["AAPL", "MSFT"]);
        list.save().unwrap();

        // Reloading with a *different* seed must not resurrect it: once the
        // file exists it is the truth, or removing a symbol would never stick.
        let reloaded =
            Watchlist::load(guard.0.join("watchlist.toml"), &["TSLA".to_string()]).unwrap();
        assert_eq!(reloaded.symbols(), ["AAPL", "MSFT"]);
    }

    #[test]
    fn symbols_are_normalised_so_the_same_ticker_cannot_appear_twice() {
        let (mut list, _g) = watchlist("dupes", &[]);
        assert!(list.add("aapl"));
        assert!(!list.add("AAPL"), "case must not create a second entry");
        assert!(!list.add("  aapl  "), "nor padding");
        assert_eq!(list.symbols(), ["AAPL"]);
    }

    #[test]
    fn blank_symbols_are_refused() {
        let (mut list, _g) = watchlist("blank", &[]);
        assert!(!list.add(""));
        assert!(!list.add("   "));
        assert!(list.symbols().is_empty());
    }

    #[test]
    fn removing_a_symbol_survives_a_reload() {
        let (mut list, guard) = watchlist("remove", &["AAPL", "MSFT"]);
        assert!(list.remove("aapl"), "removal is case-insensitive too");
        list.save().unwrap();

        let reloaded = Watchlist::load(guard.0.join("watchlist.toml"), &[]).unwrap();
        assert_eq!(reloaded.symbols(), ["MSFT"]);
    }

    #[test]
    fn an_empty_seed_writes_nothing_so_the_user_is_not_given_a_file_of_nothing() {
        let (mut list, guard) = watchlist("empty", &[]);
        list.save().unwrap();
        assert!(
            !guard.0.join("watchlist.toml").exists(),
            "nothing to persist yet"
        );
    }
}

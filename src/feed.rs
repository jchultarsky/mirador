//! Just enough RSS to answer "what happened in the world".
//!
//! Three fields out of each `<item>`: the title, the link and the date. Not the
//! description — every feed measured carries between eighty and four hundred
//! characters of real article prose there, and that is somebody's writing
//! rather than a fact about the world. A headline is a fact; a summary is a
//! work. Titles only also happens to solve the space problem, which is a nice
//! coincidence and not the reason.
//!
//! **RSS 2.0 only.** Every feed sampled while designing this was RSS 2.0 —
//! BBC, Ars Technica, NASA, Phys.org and Hacker News. Atom exists and is not
//! read; a feed that produces no items says so in the panel rather than looking
//! empty, which is the honest failure for something this narrow.
//!
//! Parsing goes through `quick-xml` rather than being hand-rolled, which is the
//! opposite of the call [`crate::ical`] made and worth explaining. iCalendar is
//! line-based and can be read with `split` and a state machine. XML has
//! entities, CDATA and namespaces, and hand-decoding those is exactly how
//! `Don&#8217;t` reaches the screen. Measured before choosing: `quick-xml` adds
//! two crates where `rss` adds twenty-three and `feed-rs` fifty-eight, against
//! the hundred and twenty-three already here.

use anyhow::{Context, Result};
use jiff::Zoned;
use quick_xml::events::Event as XmlEvent;

/// One headline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Story {
    /// The feed's display name, filled in by the panel — a feed does not know
    /// what you call it, and `BBC WORLD` reads better than the channel title.
    pub source: String,
    pub title: String,
    pub link: String,
    /// When it was published, if the feed said so in a form that parsed.
    ///
    /// The three feeds sampled emitted `GMT`, `+0000` and `EDT`, and jiff reads
    /// all three — including the obsolete zone abbreviations, which RFC 2822
    /// permits an implementation to treat as unknown and which it instead maps
    /// correctly (`EDT` to -04:00). That was checked rather than assumed; the
    /// assumption had been the other way.
    ///
    /// `None` therefore means a missing or genuinely malformed date. The story
    /// still shows, without an age — the same choice the watchlist makes for a
    /// missing price. Dropping it would hide news over a formatting slip.
    pub published: Option<Zoned>,
}

/// Pull the stories out of an RSS document.
///
/// Tolerant by design: an item with no title is skipped, an unparseable date
/// becomes `None`, and unknown elements are ignored. A feed is somebody else's
/// output and will contain things this does not expect.
pub fn parse(xml: &str) -> Result<Vec<Story>> {
    let mut reader = quick_xml::Reader::from_str(xml);
    // Deliberately *not* `trim_text(true)`. An entity splits an element's text
    // into several events, so a headline like `‘Dead stars’ may have appetites`
    // arrives as text, entity, text, entity, text — and trimming each piece
    // eats the space that followed the closing quote, producing
    // `'Dead stars'may have`. Seen on a real feed within a minute of the panel
    // first running. The assembled value is trimmed once, at the end.
    reader.config_mut().trim_text(false);

    let mut stories = Vec::new();
    let mut in_item = false;
    // Which element's text is currently being collected. `None` between them.
    let mut field: Option<Field> = None;
    let mut current = Partial::default();

    loop {
        match reader.read_event().context("reading the feed as XML")? {
            XmlEvent::Start(tag) => {
                match local_name(tag.name().as_ref()) {
                    b"item" => {
                        in_item = true;
                        current = Partial::default();
                    }
                    // Only inside an item: a feed's channel has a `<title>` and
                    // a `<link>` of its own, and reading those as a story is how
                    // the outlet's own name ends up as the first headline.
                    b"title" if in_item => field = Some(Field::Title),
                    b"link" if in_item => field = Some(Field::Link),
                    b"pubDate" if in_item => field = Some(Field::Date),
                    _ => {}
                }
            }
            XmlEvent::End(tag) => match local_name(tag.name().as_ref()) {
                b"item" => {
                    in_item = false;
                    if let Some(story) = current.take() {
                        stories.push(story);
                    }
                }
                _ => field = None,
            },
            // CDATA and plain text both reach here; `quick-xml` unescapes
            // entities for us, which is most of why it is here at all.
            XmlEvent::Text(text) => {
                if let Some(which) = field {
                    let value = text.decode().unwrap_or_default().into_owned();
                    current.set(which, &value);
                }
            }
            XmlEvent::CData(data) => {
                if let Some(which) = field {
                    let value = String::from_utf8_lossy(&data).into_owned();
                    current.set(which, &value);
                }
            }
            // `&amp;` and `&#8217;` arrive as their own events rather than as
            // part of the text around them, so ignoring these silently deletes
            // every ampersand and every typographic quote — which is how a link
            // came out as `?at_medium=RSSat_campaign=x` the first time.
            //
            // Worth noting against the choice to use a library at all: it does
            // not do this for you. What it does do is get the *splitting* right,
            // which is the part a hand-rolled scanner gets wrong.
            XmlEvent::GeneralRef(reference) => {
                if let Some(which) = field
                    && let Some(text) = resolve_entity(&reference)
                {
                    current.set(which, &text);
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }

    Ok(stories)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Link,
    Date,
}

#[derive(Debug, Default)]
struct Partial {
    title: String,
    link: String,
    date: String,
}

impl Partial {
    fn set(&mut self, which: Field, value: &str) {
        // Appended rather than assigned: an entity splits an element's text
        // into several events, so `AT&T` arrives as `AT`, `&`, `T`.
        let slot = match which {
            Field::Title => &mut self.title,
            Field::Link => &mut self.link,
            Field::Date => &mut self.date,
        };
        slot.push_str(value);
    }

    /// A story, if there is enough of one. A headline with no text is not news.
    fn take(&mut self) -> Option<Story> {
        let title = self.title.trim();
        if title.is_empty() {
            return None;
        }
        Some(Story {
            source: String::new(),
            title: title.to_string(),
            link: self.link.trim().to_string(),
            published: parse_date(self.date.trim()),
        })
    }
}

/// The text an entity reference stands for.
///
/// Numeric references are resolved by `quick-xml`. Named ones are not, because
/// XML only predefines five and anything else has to come from a DTD nobody is
/// going to fetch — so the five are spelled out here, plus `nbsp`, which feeds
/// emit constantly despite it being an HTML entity rather than an XML one.
/// Anything else is dropped rather than shown raw: `&hellip;` in a headline is
/// noise, and there is no honest way to guess.
fn resolve_entity(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    if let Ok(Some(character)) = reference.resolve_char_ref() {
        return Some(character.to_string());
    }
    let name = reference.decode().ok()?;
    Some(
        match name.as_ref() {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            "nbsp" => " ",
            _ => return None,
        }
        .to_string(),
    )
}

/// An RFC 2822 date, or `None` if the feed wrote one this cannot read.
fn parse_date(raw: &str) -> Option<Zoned> {
    if raw.is_empty() {
        return None;
    }
    jiff::fmt::rfc2822::parse(raw).ok()
}

/// The element name with any namespace prefix removed.
///
/// Feeds use `dc:date` and `media:content` freely, and matching on the whole
/// qualified name would miss anything a feed chose to namespace.
fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|byte| *byte == b':') {
        Some(colon) => &name[colon + 1..],
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from real feeds, and deliberately awkward: CDATA titles, an
    /// entity inside a link, an entity that splits a title into three text
    /// events, the three date spellings seen in the wild, an item with no date
    /// and an item with no title at all.
    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>The Feed Itself</title>
    <link>https://example.org</link>
    <item>
      <title><![CDATA[Wildfire now nine miles from Bordeaux, mayor warns]]></title>
      <description><![CDATA[Some prose that must not be read.]]></description>
      <link>https://example.org/a?at_medium=RSS&amp;at_campaign=x</link>
      <pubDate>Mon, 27 Jul 2026 17:50:42 GMT</pubDate>
    </item>
    <item>
      <title>&#8216;Dead stars&#8217; may have surprisingly healthy appetites</title>
      <link>https://example.org/f</link>
      <pubDate>Mon, 27 Jul 2026 12:00:00 GMT</pubDate>
    </item>
    <item>
      <title>AT&amp;T reverses course on the physical button</title>
      <link>https://example.org/b</link>
      <pubDate>Mon, 27 Jul 2026 15:58:34 +0000</pubDate>
    </item>
    <item>
      <title>Europa Clipper returns its first images</title>
      <link>https://example.org/c</link>
      <pubDate>Mon, 27 Jul 2026 15:40:06 EDT</pubDate>
    </item>
    <item>
      <title>A story its feed forgot to date</title>
      <link>https://example.org/d</link>
    </item>
    <item>
      <link>https://example.org/e</link>
      <pubDate>Mon, 27 Jul 2026 10:00:00 GMT</pubDate>
    </item>
  </channel>
</rss>
"#;

    #[test]
    fn the_feeds_own_title_is_not_a_story() {
        let stories = parse(SAMPLE).expect("parses");
        assert!(
            !stories.iter().any(|s| s.title == "The Feed Itself"),
            "the channel's own title became a headline: {:?}",
            stories.iter().map(|s| &s.title).collect::<Vec<_>>()
        );
        assert_eq!(stories.len(), 5, "five items have titles, one does not");
    }

    /// `quick-xml` is here rather than a hand-rolled scanner precisely for
    /// this: CDATA and entities, decoded rather than shown.
    #[test]
    fn cdata_and_entities_come_out_as_text() {
        let stories = parse(SAMPLE).expect("parses");
        assert_eq!(
            stories[0].title,
            "Wildfire now nine miles from Bordeaux, mayor warns"
        );
        assert_eq!(
            stories[0].link, "https://example.org/a?at_medium=RSS&at_campaign=x",
            "&amp; in a URL is decoded, not left as written"
        );
        assert_eq!(
            stories[2].title, "AT&T reverses course on the physical button",
            "an entity splits the text into several events and must be rejoined"
        );
    }

    /// The bug that reached the screen within a minute of the panel first
    /// running: an entity splits the text around it, and trimming each piece
    /// separately eats the space that followed. `'Dead stars'may have`.
    #[test]
    fn a_space_next_to_an_entity_survives() {
        let stories = parse(SAMPLE).expect("parses");
        let quoted = stories
            .iter()
            .find(|s| s.title.contains("Dead stars"))
            .expect("present");
        assert_eq!(
            quoted.title,
            "\u{2018}Dead stars\u{2019} may have surprisingly healthy appetites"
        );
    }

    /// Every date form seen in the wild reads, including the obsolete zone
    /// abbreviations — `EDT` becomes -04:00 rather than being discarded or,
    /// worse, silently read as UTC and shown four hours out. A story whose date
    /// is missing or malformed still appears, without an age.
    #[test]
    fn the_date_forms_feeds_actually_emit_all_read() {
        let stories = parse(SAMPLE).expect("parses");
        let by_title = |want: &str| {
            stories
                .iter()
                .find(|s| s.title.starts_with(want))
                .unwrap_or_else(|| panic!("`{want}` is missing"))
        };

        assert!(by_title("Wildfire").published.is_some(), "GMT reads");
        assert!(by_title("AT&T").published.is_some(), "+0000 reads");

        // The one worth pinning: an obsolete abbreviation must reach the right
        // offset, not be quietly taken as UTC and shown four hours out.
        let europa = by_title("Europa").published.as_ref().expect("EDT reads");
        assert_eq!(
            europa.offset().seconds(),
            -4 * 3600,
            "EDT is -04:00, not UTC"
        );

        assert!(
            by_title("A story its feed forgot").published.is_none(),
            "a missing date costs the age, not the story"
        );
    }

    #[test]
    fn something_that_is_not_a_feed_is_an_error_or_an_empty_list() {
        // Not XML at all.
        assert!(parse("<<<not xml").is_err() || parse("<<<not xml").unwrap().is_empty());
        // Valid XML, no items.
        assert!(parse("<html><body>hello</body></html>").unwrap().is_empty());
    }

    #[test]
    fn a_namespaced_element_is_matched_by_its_local_name() {
        assert_eq!(local_name(b"dc:creator"), b"creator");
        assert_eq!(local_name(b"title"), b"title");
    }
}

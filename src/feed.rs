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
//! read. An Atom document is refused by name, and a document with neither a
//! `<channel>` nor an `<item>` — a web page, an error page — is an error too,
//! so either reaches the panel as a failure with its reason rather than as a
//! quiet day, which is what both did until 2026-10-07, while the README claimed
//! Atom was read. An RSS channel with no items is a feed with nothing in it,
//! and reads as one.
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

/// What one feed yielded.
#[derive(Debug, Default)]
pub struct Feed {
    pub stories: Vec<Story>,
    /// Why reading stopped before the end of the document, when it did.
    pub fault: Option<String>,
}

/// Pull the stories out of an RSS document.
///
/// Tolerant by design: an item with no title is skipped, an unparseable date
/// becomes `None`, and unknown elements are ignored. A feed is somebody else's
/// output and will contain things this does not expect.
pub fn parse(xml: &str) -> Result<Feed> {
    let mut reader = quick_xml::Reader::from_str(xml);
    // Deliberately *not* `trim_text(true)`. An entity splits an element's text
    // into several events, so a headline like `‘Dead stars’ may have appetites`
    // arrives as text, entity, text, entity, text — and trimming each piece
    // eats the space that followed the closing quote, producing
    // `'Dead stars'may have`. Seen on a real feed within a minute of the panel
    // first running. The assembled value is trimmed once, at the end.
    reader.config_mut().trim_text(false);
    // A bare `&` — `AT&T` as the publisher typed it — is the commonest fault in
    // a real feed, and by default quick-xml refuses the whole document for it.
    // It is plainly an ampersand, so it is read as one. The flag covers an `&`
    // with no `;` before the next `<` or `&`; one *with* a `;` after it reaches
    // `resolve_entity` as a reference, which gives it back as written.
    reader.config_mut().allow_dangling_amp = true;

    let mut stories = Vec::new();
    let mut in_item = false;
    let mut root_seen = false;
    let mut saw_item = false;
    let mut saw_channel = false;
    // Which element's text is currently being collected. `None` between them.
    let mut field: Option<Field> = None;
    let mut current = Partial::default();

    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            // Broken part-way down. The stories before the fault were read
            // whole and are still news, so they are kept — and the fault goes
            // with them rather than being swallowed, because a feed that stops
            // reading at its third item is still a feed that is broken.
            Err(e) if !stories.is_empty() => {
                return Ok(Feed {
                    stories,
                    fault: Some(format!("partly unreadable: {e}")),
                });
            }
            Err(e) => return Err(e).context("reading the feed as XML"),
        };
        match event {
            XmlEvent::Start(tag) => {
                let root = !std::mem::replace(&mut root_seen, true);
                match local_name(tag.name().as_ref()) {
                    // Atom is valid XML with no `<item>` in it, so without this
                    // it came back as an empty list and the panel called it a
                    // quiet day. Named, so the reason reaches the panel.
                    "feed" if root => {
                        anyhow::bail!("an Atom feed; mirador reads RSS 2.0 only");
                    }
                    "item" => {
                        in_item = true;
                        saw_item = true;
                        current = Partial::default();
                    }
                    "channel" => saw_channel = true,
                    // Only inside an item: a feed's channel has a `<title>` and
                    // a `<link>` of its own, and reading those as a story is how
                    // the outlet's own name ends up as the first headline.
                    "title" if in_item => field = Some(Field::Title),
                    "link" if in_item => field = Some(Field::Link),
                    "pubDate" if in_item => field = Some(Field::Date),
                    _ => {}
                }
            }
            XmlEvent::End(tag) => match local_name(tag.name().as_ref()) {
                "item" => {
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
                    // 0.42 validates UTF-8 in the reader and hands events out
                    // as `str`, so the old decode step has no failure left to
                    // handle — the event *is* the text.
                    current.set(which, text.as_ref());
                }
            }
            XmlEvent::CData(data) => {
                if let Some(which) = field {
                    current.set(which, data.as_ref());
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
                if let Some(which) = field {
                    current.set(which, &resolve_entity(&reference));
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }

    // A document with no `<channel>` and no `<item>` is not a feed with nothing
    // in it — it is a web page, an error page, or a format this does not read,
    // and an empty list would reach the panel as a quiet day. A channel with
    // no items is a real feed on a quiet day, and says so.
    if !saw_item && !saw_channel {
        anyhow::bail!("not an RSS feed: no <channel> or <item> in it");
    }

    Ok(Feed {
        stories,
        fault: None,
    })
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
            title: clip(title, MAX_TITLE),
            link: clip(self.link.trim(), MAX_LINK),
            published: parse_date(self.date.trim()),
        })
    }
}

/// The longest headline kept. Real ones run to about 150 characters; this is
/// generous enough that nothing genuine is ever clipped.
const MAX_TITLE: usize = 400;

/// The longest link kept. Real ones are well under 200.
const MAX_LINK: usize = 1_000;

/// The longest feed name kept, applied where the panel names the feed.
pub const MAX_SOURCE: usize = 80;

/// Keep the first `limit` characters of `text`.
///
/// This is the bound that was missing, and the reason it matters is not tidiness
/// — it is that everything downstream is per *frame*. A story's title is wrapped
/// on every draw and its source is uppercased on every draw, so an unbounded
/// string here becomes unbounded work sixty times a minute. Measured before
/// fixing: a 2 MB headline wrapped to 40,000 lines and cost **72 ms a frame**,
/// and `ureq`'s body cap allows five times that. A feed you do not control
/// should not be able to decide how much work the dashboard does.
///
/// Cutting on a character boundary rather than a byte one, so a clipped headline
/// is still a string. The panel truncates again for display, in *cells*; this
/// bound is about what is worth holding at all.
fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect()
}

/// The text an entity reference stands for.
///
/// Numeric references are resolved by `quick-xml`. Named ones are not, because
/// XML only predefines five and anything else has to come from a DTD nobody is
/// going to fetch — so the five are spelled out here, plus `nbsp`, which feeds
/// emit constantly despite it being an HTML entity rather than an XML one.
///
/// Anything else is given back as it was written, `&` and `;` included. It used
/// to be dropped, on the theory that `&hellip;` in a headline is noise — but
/// what reaches here is not always a reference. A bare `&` with a `;` later in
/// the same text is read by quick-xml as a reference *up to* the `;`, so
/// `R&D spending rises; analysts wary` arrives as `R`, a reference named
/// `D spending rises`, and ` analysts wary`, and dropping the name deleted the
/// middle of the headline with nothing to say so. There is no honest way to
/// guess which is which, and showing what the feed wrote never loses a word.
fn resolve_entity(reference: &quick_xml::events::BytesRef<'_>) -> String {
    if let Ok(Some(character)) = reference.resolve_char_ref() {
        return character.to_string();
    }
    match reference.as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        name => return format!("&{name};"),
    }
    .to_string()
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
fn local_name(name: &str) -> &str {
    match name.rfind(':') {
        Some(colon) => &name[colon + 1..],
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bound that was missing. Everything downstream of a story runs per
    /// *frame* — the title is wrapped on every draw, the source uppercased on
    /// every draw — so an unbounded field here is unbounded work sixty times a
    /// minute. Measured before the fix: a 2 MB headline wrapped to 40,000 lines
    /// and cost 72 ms a frame, and `ureq`'s body cap allows five times that.
    ///
    /// A feed is the one input to this program that somebody else writes.
    #[test]
    fn a_hostile_feed_cannot_decide_how_much_work_the_dashboard_does() {
        let huge = "word ".repeat(400_000);
        let xml = format!(
            "<rss><channel><item><title>{huge}</title>\
             <link>http://example.com/{huge}</link></item></channel></rss>"
        );
        let stories = parse(&xml).expect("parses").stories;
        let story = stories.first().expect("one story");

        assert!(
            story.title.chars().count() <= MAX_TITLE,
            "a {} character headline survived",
            story.title.chars().count()
        );
        assert!(
            story.link.chars().count() <= MAX_LINK,
            "a {} character link survived",
            story.link.chars().count()
        );

        // The property that actually matters: what it costs to draw.
        let wrapped = crate::grid::wrap(&story.title, 50);
        assert!(
            wrapped.len() <= 40,
            "the headline still wraps to {} lines a frame",
            wrapped.len()
        );
    }

    /// And an ordinary headline is untouched — a bound that clips real news is
    /// worse than no bound.
    #[test]
    fn a_headline_of_ordinary_length_is_not_clipped() {
        let title = "Astronomers find a planet where it rains glass sideways, \
                     and the discovery may rewrite how we think gas giants form";
        let xml = format!("<rss><channel><item><title>{title}</title></item></channel></rss>");
        let stories = parse(&xml).expect("parses").stories;
        assert_eq!(stories[0].title, title, "a real headline was clipped");
    }

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
        let stories = parse(SAMPLE).expect("parses").stories;
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
        let stories = parse(SAMPLE).expect("parses").stories;
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
        let stories = parse(SAMPLE).expect("parses").stories;
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
        let stories = parse(SAMPLE).expect("parses").stories;
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

    /// Something that is not an RSS feed is an error that says what it is,
    /// never an empty list. An empty list reaches the panel as a quiet day, and
    /// a reader looking at `No stories.` has no way to learn that the address
    /// they configured is a web page.
    #[test]
    fn something_that_is_not_a_feed_is_an_error_naming_the_cause() {
        // Not XML at all.
        assert!(parse("<<<not xml").is_err());
        // Valid XML, no items.
        let err = parse("<html><body>hello</body></html>").expect_err("no items is not a feed");
        assert!(format!("{err:#}").contains("<item>"), "got `{err:#}`");
    }

    /// An RSS channel with no items is a feed on a quiet day, not a broken one;
    /// refusing it would put an outlet that published nothing today under
    /// "Cannot read the feeds".
    #[test]
    fn an_empty_channel_is_an_empty_feed_not_a_failure() {
        let feed = parse("<rss version=\"2.0\"><channel><title>Quiet</title></channel></rss>")
            .expect("an empty channel is still a feed");
        assert!(feed.stories.is_empty(), "{:?}", feed.stories);
        assert_eq!(feed.fault, None);
    }

    /// The README said Atom was read, and the parser never had an arm for
    /// `<entry>`: an Atom document is valid XML holding no `<item>`, so it came
    /// back as an empty list and the panel called it a quiet day. Refused by
    /// name instead, so the reason reaches the panel's failure line.
    #[test]
    fn an_atom_feed_says_so_rather_than_looking_empty() {
        let atom = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Releases</title>
  <entry>
    <title>v1.2.0</title>
    <link href="https://example.org/releases/v1.2.0"/>
    <updated>2026-10-01T12:00:00Z</updated>
  </entry>
</feed>"#;
        let err = parse(atom).expect_err("Atom is not read");
        assert!(
            format!("{err:#}").contains("Atom"),
            "the failure should name the cause, got `{err:#}`"
        );
    }

    /// A bare `&` — `AT&T` as the publisher typed it — is the commonest fault in
    /// a real feed, and quick-xml refuses the whole document for it by default.
    /// It is plainly an ampersand, and is read as one.
    #[test]
    fn a_bare_ampersand_is_read_as_one() {
        let xml = SAMPLE.replace("AT&amp;T", "AT&T");
        let feed = parse(&xml).expect("a bare `&` is not worth the whole feed");
        assert_eq!(feed.stories.len(), 5, "every story survives");
        assert_eq!(
            feed.stories[2].title,
            "AT&T reverses course on the physical button"
        );
        assert_eq!(feed.fault, None, "nothing was lost, so nothing is reported");
    }

    /// The flag above covers an `&` with no `;` before the next `<` or `&`.
    /// When a `;` comes first, quick-xml reads everything up to it as the name
    /// of a reference whatever the flag says, and a name this does not know was
    /// dropped — taking the text between with it, in silence. `R&D spending
    /// rises; analysts wary` came out as `R analysts wary`, and a link with a
    /// `;` in its query came out as a different address that was then
    /// linkified and opened. A name that is not one of the six is given back
    /// as it was written.
    #[test]
    fn a_bare_ampersand_before_a_semicolon_keeps_the_text_between() {
        let xml = "<rss><channel>\
            <item><title>R&D spending rises; analysts wary</title>\
                  <link>https://example.com/a?id=1&amp;x=2&page=3;view=full</link></item>\
            <item><title>Q&A: why it matters; more</title></item>\
            <item><title>AT&T; Verizon merge</title></item>\
            <item><title>Wait&hellip; what</title></item>\
            </channel></rss>";
        let feed = parse(xml).expect("parses");
        let titles: Vec<&str> = feed.stories.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "R&D spending rises; analysts wary",
                "Q&A: why it matters; more",
                "AT&T; Verizon merge",
                "Wait&hellip; what",
            ],
            "the text between an `&` and a later `;` is part of the headline"
        );
        assert_eq!(
            feed.stories[0].link, "https://example.com/a?id=1&x=2&page=3;view=full",
            "a link must reach the panel as the address it was"
        );
        assert_eq!(feed.fault, None, "nothing was lost, so nothing is reported");
    }

    /// One ill-formed tag in the third item used to discard the two stories
    /// read whole before it. They are kept, and the fault rides alongside them
    /// so the panel can still say the feed did not read cleanly.
    #[test]
    fn a_fault_part_way_down_keeps_the_stories_before_it() {
        let xml = SAMPLE.replace(
            "<title>AT&amp;T reverses course on the physical button</title>",
            "<title>AT&amp;T reverses course on the physical button</titel>",
        );
        let feed = parse(&xml).expect("two good stories are still news");
        let titles: Vec<&str> = feed.stories.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Wildfire now nine miles from Bordeaux, mayor warns",
                "\u{2018}Dead stars\u{2019} may have surprisingly healthy appetites",
            ],
            "the stories before the fault, and nothing after it"
        );
        assert!(
            feed.fault.as_deref().is_some_and(|f| f.contains("partly")),
            "the fault must be reported, not swallowed: {:?}",
            feed.fault
        );

        // A fault before any story has nothing to keep, and is still an error.
        assert!(parse("<rss><channel><item><title>x</titel></item></channel></rss>").is_err());
    }

    #[test]
    fn a_namespaced_element_is_matched_by_its_local_name() {
        assert_eq!(local_name("dc:creator"), "creator");
        assert_eq!(local_name("title"), "title");
    }
}

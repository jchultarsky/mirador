# CLAUDE.md — working notes for mirador

Context for anyone (human or agent) picking this repo up cold. Design rationale
that belongs to users lives in `README.md` and `CONTRIBUTING.md`; this file is
the stuff you would otherwise have to reverse-engineer from the diff.

There was a `HANDOFF.md` alongside this carrying the perishable half — where
work stopped, what was waiting on a decision. Its open items are all either
done or folded in below, so it has been deleted as it asked to be. Write
another the same way if you hand off mid-flight, and delete it the same way.

## What this is

A personal information dashboard for the terminal, in Rust + ratatui. Clock,
weather, tasks, and live system metrics in a config-driven grid. MIT licensed.
Repo: `github.com/jchultarsky/mirador`. Owner: Julian Chultarsky.

A *mirador* is a lookout — the tower you climb to see everything at once.

**Known name collision, accepted knowingly:** Project Mirador, the IIIF image
viewer. Raised before the name was chosen; not a reason to revisit it.

## The one-line pitch that shapes every decision

A tab you leave open all day and come back to. That constrains more than it
sounds like it does: no ambient blinking, no shimmering graphs, no doomscroll
hooks, calm by default so that *not* calm is legible at a glance.

## Commands

```sh
cargo test                                    # all fast, no network, no fixtures
cargo clippy --all-targets -- -D warnings     # must be silent
cargo fmt --all -- --check                    # must be silent
RUSTDOCFLAGS="-D warnings" \
  cargo doc --no-deps --document-private-items  # CI runs this; it caught a
                                                # broken intra-doc link that
                                                # the other three did not
cargo run -- --print-config > /tmp/m.toml     # scratch config to experiment on
cargo run -- --config /tmp/m.toml
```

**The bar is zero warnings and zero errors across all four.** Running only the
first three is how a red build reached `main`: rustdoc rejects a link to a
`cfg(test)` item, and nothing else notices.

`rust-version` is driven by dependencies, and `cargo` reports only the *first*
blocker — chasing them one at a time costs a CI round trip each. Get the real
floor in one go:

```sh
cargo metadata --format-version 1 | \
  jq -r '[.packages[].rust_version | select(.)] | max'
```

It currently comes from `sysinfo`, not from anything this crate does. Five
places have to agree: `Cargo.toml`, `.github/workflows/ci.yml` (twice), the
README and CONTRIBUTING.

Note that `rust-version` also changes what clippy suggests — raising
the MSRV to 1.88 turned every nested `if let` into a `collapsible_if` error,
because let-chains only became available there, and 1.88 → 1.95 turned every
`Duration::from_secs(3600)` into one, because `from_hours` had stabilised.
Expect a round of mechanical fixes with any bump, and do not take the
suggestion on faith: clippy will offer `from_days`, which is *still unstable*
at the 1.95 floor and would break the very job the bump was fixing.

This cuts the other way too, and it is worth knowing before you go hunting.
**Clippy gating means a local run and CI can legitimately disagree**: bump
`rust-version` locally, run clippy, and you will see lints a CI still on the
old floor does not. That is the gate working, not CI failing. Compare the
`rust-version` each side is using before concluding anything is broken.

The crate enables `clippy::pedantic`. When a lint is genuinely wrong, add a
targeted `#[allow]` *with a comment saying why* — do not widen the allow list
in `Cargo.toml`.

**`assets/default_config.toml` is `include_str!`-baked into the binary.**
Editing it does nothing until you rebuild. This has twice looked like a change
that did not land when it simply had not been compiled.

To eyeball the rendering without a terminal:

```sh
cargo test dump_dashboard -- --ignored --nocapture   # renders to stdout
```

To drive it in a real terminal headlessly, run it under `tmux` and
`tmux capture-pane -p`. Several layout bugs only showed up this way — the
`TestBackend` dump will not catch a panel whose content is pushed off the
bottom, because nothing errors. `-e` keeps the escape sequences, which is the
only way to prove a colour bug; a capture of black-on-black shows nothing
either way.

**A test that cannot fail is documentation with a `#[test]` on it.** The id
reuse bug shipped with `ids_are_unique_and_survive_deletion` sitting directly
on top of it: the test compared the new id against the *surviving* task rather
than the *removed* one, so it passed throughout. When you fix a bug that had a
test nearby, check that the test would have caught it — break the fix on
purpose and watch it go red. Twice now that check was the difference between a
real test and a reassuring one.

## Architecture

```
main.rs      CLI parsing, terminal setup
app.rs       event loop, focus ring, grid geometry, help overlay, status bar
panel.rs     the Panel trait — the seam every widget goes through (in-tree)
frame.rs     panel frames, Binding type, key hints punched into borders
grid.rs      shared column grid with named headers
chart.rs     braille graphs + baked colour gradients
glyphs.rs    block numerals, bold-uppercase labels, weather art
theme.rs     colours and gradient stops
themes.rs    `theme = "name"`: bundled themes, inherits, palettes
config/      mod.rs: paths, loading, validation; widgets.rs: one settings
             struct per panel; layout.rs: the grid
picker.rs    the `w` dialog — owns its cursor, returns an Action to the shell
arrange.rs   the `m` mode's arithmetic: where a panel goes when you move it
prompt.rs    the one-line question a panel asks for a path, place or zone,
             with an optional list to choose from
zones.rs     world clocks as a data file, plus the city→zone table
migrate.rs   textual in-place upgrade of configs written by older versions
layout_edit.rs surgical `[layout]` rewrites, so panel changes reach the config
store.rs     write_atomic — every file mirador owns goes through it
state.rs     UI-changed preferences, remembered across restarts
update.rs    the opt-in update check — off unless the config turns it on
watch.rs     the watch log's events and the rule that decides what is one
feed.rs      enough RSS for a headline; quick-xml, unlike ical.rs — see below
task.rs      task model + TOML store
note.rs      note model + TOML store
quote.rs     Quote + the pluggable QuoteSource trait + the watchlist store
poll.rs      the sliced sleep the two fetch threads share
samples.rs   the bounded history behind the cpu and network graphs
selection.rs list cursor movement and click-to-row
textfield.rs single-line text editor used by task entry
textarea.rs  multi-line text editor used by note bodies
dateinput.rs due-date entry
ical.rs      enough RFC 5545 to answer "what is next"; no new dependencies
widgets/     clocks, weather, todo, notes, stocks, calendar, agenda,
             pomodoro, watchlog, news, cpu, network
```

`Panel` has two input hooks. `handle_key` goes to the *focused* panel;
`handle_mouse` goes to the panel under the *pointer*, which is deliberately not
the same thing — a scroll wheel must move the list it is aimed at without
yanking the keyboard away from what the user was typing in.

Adding a widget: implement `Panel`, add a config struct to `config/widgets.rs`,
add the name to `WIDGET_NAMES` and an arm to `build()` in `widgets/mod.rs`,
document it in `assets/default_config.toml` and the README. Nothing else needs
to know. `CONTRIBUTING.md` has the same list with more detail — this one is the
map, that one is the procedure.

## Invariants — do not break these

1. **Panels must never block.** Network and disk I/O go on a background thread
   and are polled in `tick()`; see `widgets/weather.rs`. A blocking panel
   freezes the whole dashboard.
2. **`Panel::captures_input()` is an absolute veto on global keys.** It is what
   stops typing `q` into a task title from quitting. Ctrl+C is the sole
   exception, and it is safe because panels save as they go.
3. **One `Binding` declaration feeds three surfaces** — border hint, status bar,
   help overlay. Never hardcode hint text anywhere else; it will drift.
4. **Hints are shown only for the focused panel.** A flat list of every binding
   teaches people to press panel keys while the wrong panel is focused.
5. **Grid columns are resolved once per draw for the whole list**, not per row.
   Per-row was a real bug: a task with no due date slid every row below it out
   of alignment.
6. **Every column of a header gets the same treatment.** Mixed treatment reads
   as an accident (`DONE PRI T A S K`). This used to need real machinery, since
   a letterspaced label could overflow a narrow column and demote the whole row;
   the bold face costs no extra width, so the question no longer arises.
7. **Task writes are atomic** (temp file + rename) and save failures surface in
   the panel. A silent failed save on a task list is unforgivable.
8. **Body text is `Color::Reset`.** Hard-coding a foreground fights the user's
   own terminal theme.
9. **Measure text in display cells, never in `chars()`.** `grid.rs` uses
   `unicode-width`. This is not pedantry: a glyph like `☀` or an emoji occupies
   two cells, so counting characters silently shifts every column after it and
   values end up under the wrong headers. This was a real, shipped bug.
10. **Sky marks must measure the width they draw.** Several obvious weather
    emoji — U+1F327 rain cloud, U+1F328 snow cloud, U+2601 cloud, U+1F32B fog —
    report width 1 from `unicode-width` but render as two cells. Only glyphs
    that measure 2 *and* draw 2 are used. `every_sky_mark_has_a_predictable_display_width`
    asserts this; if you swap a glyph and that test fails, believe it.
11. **Never render an empty cell in a table.** A blank reads as "this column is
    broken"; `0%` reads as "it is not going to rain", which is the fact the
    reader wanted. Use `–` for genuinely unknown.
12. **A config number that describes content is a floor, not a ceiling.**
    `[cpu].history = 120` capped the graph at 60 cells, so a wider panel drew
    dead space; the buffer now grows to the width. Before adding a count to a
    config, ask what happens when the panel is bigger than the count implies.
13. **Anything time-dependent must re-derive itself on a tick.** The dashboard
    runs for days: a `today` captured at construction silently rots, and
    overdue tasks stop being red. `todo`, `notes` and `calendar` all re-read the
    date in `tick()` and rebuild when it rolls over.
14. **A fetch failure must not discard good data — except for prices.** Weather
    keeps its last reading and shows its age, because old weather is useful and
    a blank panel is not. Quotes do the opposite and fall back to `–`: a stale
    price read as live is worse than no price, which is the same reasoning that
    keeps quotes out of any file on disk.
15. **A panel that cannot use more space must say so**, via `Panel::max_width`
    / `max_height`. Otherwise proportional layout hands it space it cannot use
    while a list next door runs out. Return `None` for anything that scrolls or
    scales; return a figure only when more space genuinely buys the reader
    nothing. Both are whole-panel measurements, frame and padding included.
16. **The config is edited, never reserialised.** This is the real form of the
    "never rewrites the config" rule, which was always about comments: a round
    trip through `toml` discards all 254 of them, including the ones mirador
    wrote to explain its own options. `migrate.rs` established the alternative
    and `layout_edit.rs` follows it — find the line, change that line, leave
    everything else alone. Adding a panel is a one-line diff.

    What makes it safe to do at all is the check at the end of
    `layout_edit::apply`: the edited text is parsed and compared against the
    layout that was requested, and a mismatch throws the edit away. So an
    unusually formatted config fails as "your change did not stick", reported
    in the picker, rather than as a mangled file.

    The split is by *what*, not by convenience: `[layout]` goes to the config
    because people read and curate it, and everything else goes to `state.rs`
    because nobody keeps their preferred sort order under version control.
17. **A remembered preference is the difference between the panels and the
    config.** `Panel::remember` reports current values unconditionally;
    `UiState::only_changes_from` drops whatever matches
    `UiState::from_config(config_as_loaded)`. The baseline has to be taken
    *before* `apply_state`, or it already contains last session's changes and
    nothing can be seen to differ from it.

    Two wrong versions were shipped before this one, and both looked right:

    - Reporting current values with no comparison pinned every panel's config
      values into the state file on the first keystroke anywhere, after which
      editing the config silently stopped working.
    - Having each panel compare against the value it was *built* with fixed
      that, and made a preference impossible to *un*-set: after `apply_state`
      the panel is built from the remembered value, so a setting toggled back
      to what the config says looked unchanged, wrote nothing, and left the
      earlier entry standing for ever.

    Both passed their unit tests. The first was caught by running it; the second
    by an adversarial review reading the data flow. Keep the comparison in one
    place, against the config, and it is symmetric by construction.

18. **First run must not be blank, and the defaults must agree.** The task and
    notes stores seed examples via `load_or_seed`, keyed on the file being
    *absent* — never on it being empty, or deleting the examples would not
    stick. The seeded titles are instructions, so they have to fit the task
    column at an ordinary width; `seeded_titles_fit_the_task_column_at_an_ordinary_width`
    pins that at 25 cells, which is what the default layout leaves at 120
    columns. Separately, `Layout::default()` and the `[layout]` block in
    `assets/default_config.toml` must describe the same dashboard — they are
    reached by different routes, and when they diverged, a config without a
    `[layout]` section silently lost three panels.

    Note for tests: constructing `TodoPanel`/`NotesPanel` against a path with
    no file now seeds it. Panel tests write an empty file first, which is the
    branch every run after the first one takes.

## Visual system

Design thesis: *the watch station*. The vernacular is a lookout's instrument
panel — chronometer, weather glass, watch log.

- **Palette:** brass `#d7af87` (instruments, focus, clock), verdigris `#5f8787`
  (engraved labels, tags), slate `#3a3a3a` (chrome). Signals: red `#d75f5f`,
  amber `#d7af5f`, moss `#87af5f`.
- **Three manufactured typefaces**, since a terminal has one font: block
  numerals (display), the terminal's own face (body), **bold uppercase**
  (utility/labels, via `glyphs::utility`). Weight and case separate label from
  data without relying on colour and without costing width.

  **Reversed decision:** the utility face was letterspaced — `N E X T  H O U R S`
  — on the theory that tracking reads as an engraved instrument label. The owner
  rejected it on sight: it reads as stretched, not engraved. Do not reintroduce
  it. Tracking also more than doubled every label, which is why the grid header
  needed all-or-nothing demotion logic; that logic is gone with it.
- **Focus by recession** — dim the unfocused, never brighten the focused, so
  exactly one thing is at full brightness.
- **The frame is a widget bus**, not decoration. Titles, jump keys, counters and
  hints are punched into the border with `┤label├`, costing zero interior rows.

### Techniques borrowed, and why

From **bpytop/btop**: braille 5×5 level table (2 samples per cell, 4 levels per
row); gradient runs **vertically by magnitude**, one colour per row — this is
deliberate and worth defending, a static colour profile means the graph does not
shimmer as data scrolls; three-stop gradients baked to a 101-entry table with
`start` dark and desaturated so idle graphs recede; the same ramp colours graph,
meter and number together; always paint a track (`⣀` baseline, `■` meter tail)
so the footprint never changes.

From **clock-tui**: run-length row glyph encoding plus an integer scale
multiplier — 13 glyphs in ~20 lines and free scaling, much better than
hand-drawn string art.

From **gitui/lazygit/ratatui's own tutorial**: focus by dimming; hints in the
bottom border; jump key in the title.

## Product decisions already settled

**The four things the dashboard exists to answer:** what time/day is it, what
tasks are next, what is the portfolio doing, what compute is available.

**The pomodoro panel answers none of them, and is here anyway** — the owner
asked for it directly. Worth recording rather than leaving the next reader to
wonder whether the filter was forgotten. It is arguably a fifth question, "how
is this stretch of work going", and it is the only panel that is *about* the
time you spend at the dashboard rather than about something outside it. That
makes it the one panel where a nagging design would do real damage, which is
why the chime defaults off and a paused timer greys out instead of blinking.
The filter still holds for anything not asked for by name.

**Identified gaps, agreed:** (1) calendar / next event is the biggest hole —
tasks are self-paced, a meeting is externally imposed and time-critical;
(2) nothing shows what *changed* since you last looked; (3) no single "is
anything on fire" signal.

**Rejected:** unread email/message counts. Looks like information, is a
doomscroll hook, turns a calm dashboard into a nagging one.

**Calendar must be independent** — no connecting to the user's mail or calendar
server. Read a local `.ics` file or a plain events file.

**Weather deliberately gets less room** than it originally had; it was not among
the four.

**No audio library, ever.** The pomodoro chime is the terminal bell, `\x07`.
Playing a sound file means talking to the platform's audio stack — `rodio`
reaches `cpal` reaches `alsa-sys`, a C library wanting dev headers on every
Linux builder, which would undo the work that got musl and Windows building at
all. The bell costs nothing and defers to settings the user has already made.
Anyone wanting a specific sound names a player in `chime_command`, which is run
directly rather than through a shell. If a future panel wants a notification,
it gets the same answer.

**Sound is off by default and so is anything else that interrupts.** The one
line that decides this: a tab you leave open all day has no business making
noise you did not ask for.

## Stock watchlist — built, in `quote.rs` and `widgets/stocks.rs`

Yahoo Finance `/v8/finance/chart/?range=1d&interval=5m`. Keyless, and one GET
returns the price, previous close, currency *and* the intraday series, so a row
costs exactly one request. **Requires a browser `User-Agent`** or you get HTTP
429 regardless of rate.

- `/v7/finance/quote` is effectively dead (401 without cookie+crumb). Stay on v8.
- Yahoo gates on **IP reputation**, not auth. Datacenter and VPN IPs are blocked
  wholesale. **This is why `QuoteSource` is a trait** — load-bearing, not
  gold-plating: the same build works on a laptop and fails on a VPS, and only a
  different source fixes it. Add one by implementing the trait and adding a name
  to `SOURCE_NAMES` and an arm to `source_for`.
- Unbuilt fallbacks: CNBC quote (batches N tickers in one request, no
  sparkline), Nasdaq chart (sparkline, self-declares delayed). Keyed: Tiingo.
- **Do not use Finnhub** — its ToS restricts *every* tier including paid to
  personal use, so no user can ever become compliant. FMP bans display in
  software products. Alpha Vantage is 25 req/day.
- Never bundle a shared API key; do not persist quotes beyond the session; poll
  ≥60s and stagger requests rather than firing concurrently. All four are
  enforced in code, not just documented.
- **The watchlist is a data file, not config.** That is the only reason the
  panel can add and remove symbols: mirador never rewrites the config, so
  `[stocks].symbols` seeds the first run and nothing after it. Reach for the
  same trick for any other user-editable list.
- `parse_chart` is split from the HTTP call so it is tested against captured
  JSON. **No test in this repo touches the network** — keep it that way.

## Named themes — built

`theme = "name"` resolves through `themes.rs`. Four themes are `include_str!`-
baked (`default`, `default-light`, `high-contrast`, `ansi`) and anything in
`<config>/mirador/themes/<name>.toml` is found first, so a bundled theme can be
replaced without renaming it. `inherits` chains with cycle detection; `[palette]`
names colours once, and a child redefining a palette entry recolours the keys
its *parent* set with it — which is why palettes are merged separately from the
colour keys rather than as each document is read.

**Two items from the original design were deliberately not built**, and the
reasons matter more than the decision:

- **Dotted-scope fallback** (`ui.border.focused` → `ui.border` → `ui`). Helix
  needs it because it has hundreds of syntax scopes and no theme can name them
  all. mirador has thirteen flat semantic keys that a theme can reasonably set
  in full, and `inherits` already covers the rest. The stated benefit — that it
  "makes `border_focused` stop being a special case" — is one field.
- **Full style objects** (fg/bg/modifiers per key). All ~240 theme reads take a
  flat `Color`, nothing wants a themed background (invariant 8), and bold and
  reversed are decided by the widget that knows what it is emphasising.

Both are additions if wanted, not corrections.

**The compat shim is a hand-written visitor, not `#[serde(untagged)]`,** and it
has to stay that way. An untagged enum reports "data did not match any variant"
for every failure, which would destroy both the misspelled-key report and the
`--migrate-config` hint for the pre-0.1.0 `rx`/`tx` keys. The map arm delegates
to the derived impl, so all of that survives; two tests pin it.

**The TOML ordering trap is real and is guarded.** Colour keys written *after*
`[palette]` land inside it and set nothing — nothing is misspelled, nothing
fails to parse, and the theme silently comes out as the defaults. The shipped
`default.toml` was written that way and the test comparing it against
`Theme::default()` **passed**, because a file that sets nothing resolves to
exactly the default. Hence two things: `check_palette_ordering` refuses such a
file by name, and the drift test is paired with one that asserts a standalone
theme *sets every key*, which is the property that can actually fail.

## The "is anything on fire" signal — built

One line in the status bar naming the single most pressing thing, absent
otherwise. `Panel::alert` is the present-tense sibling of `Panel::events`: same
notion of notable, different tense, which is why they are separate hooks.

**The threshold is "will this get worse if nobody acts in the next few
minutes",** and holding that line is the whole feature. An overdue task does not
qualify — notable, already red in its own panel, and identically overdue in an
hour. A signal lit for it is lit permanently, and a permanently lit alarm is
furniture: the unread-badge failure reached from the other direction. Eight of
the twelve panels already show what is wrong in their own frames, so the gap
this fills was aggregation, never detection.

**No all-clear, ever.** An indicator saying everything is fine is a light you
have to read to learn nothing. Absence is the signal; there is a test.

**It takes the whole bar rather than sharing it.** Squeezing it in beside the
key hints truncated the *reason*, which is the actionable half —
`read-only file syste…`. The keys are behind `?`, and an alert is gone the
moment its cause is.

**`layout_error` was the one genuine silent failure in the program.** It was
reported inside the `w` picker and nowhere else, so a rearrangement that failed
to persist said nothing once the picker closed and was gone at the next launch.
That is now an alert.

## The news panel — built

The one that came closest to the feature this dashboard turned down. Unread
counts were rejected as a doomscroll hook; news *is* the doomscroll surface.
It survives on the same kind of commitment the watch log made, and the same
three words apply: **no count, no unread state, nothing to dismiss**, plus a
fourth — **no scrolling**. It shows what fits. You cannot work through it
because there is nothing to work through. A `3 new` counter in that frame turns
it back into the rejected feature; there is a test whose failure message says so.

**Stories are interleaved across feeds, not sorted by date.** Pure date order
hands the whole visible window to whichever outlet publishes most often — the
first real run showed three consecutive Phys.org stories. Round-robin puts the
newest from each feed at the top, which is what "a window on the world" has to
mean.

**`quick-xml`, where `ical.rs` hand-rolled.** Not an inconsistency: iCalendar is
line-based and yields to `split`, where XML has entities, CDATA and namespaces.
Measured before choosing — `quick-xml` adds two crates, `rss` twenty-three,
`feed-rs` fifty-eight.

**Do not set `trim_text(true)` on the reader.** An entity splits an element's
text into several events, and trimming each piece separately eats the space
beside it: a real headline came out as `'Dead stars'may have`. The assembled
value is trimmed once, at the end. There is a test.

**Headlines only.** Every feed sampled carries 80–384 characters of article
prose in `<description>`. A headline is a fact; a summary is somebody's work.

**The shipped feeds are science, space and technology.** Choosing outlets for
general or political news is an editorial act this project should not make on
a user's behalf, and it is the kind that arrives in the issue tracker.

## The watch log — built

Fills the third instrument the design thesis named and nothing had ever
occupied. `watch.rs` holds the log and the rule for what belongs in it;
`widgets/watchlog.rs` draws it; panels report through `Panel::events()`,
drained each tick beside `tick()`.

**Three commitments are what keep it on the right side of the unread-count
rejection, and all three are presentational:** no counter in the frame, no
unread state, and no effect on any other panel's appearance. Read the original
objection closely — "looks like information, is a doomscroll hook, turns a calm
dashboard into a nagging one" — and it is about the *badge*, not about
surfacing change. Give this panel a `3 new` counter and it becomes the feature
that was turned down. There is a test whose failure message says so.

**"Since I last looked" is unknowable and the design admits it.** The dashboard
cannot see you looking, and the moments it most needs to be right are the
glances that touch nothing. mirador enables terminal focus reporting, which is
the closest honest signal, and falls back to the last keypress. Where neither
has fired it draws *no* rule line — a line in the wrong place makes a claim,
where a missing one merely says nothing.

**Focus reporting is unverified in practice.** It could not be tested here: a
headless tmux server has no attached client to have focus, so the probe could
not distinguish "unsupported" from "nothing is focused". The keypress fallback
carries the feature until someone confirms it on a real terminal, and tmux
needs `focus-events on` regardless.

**The log does not persist, on purpose.** A file would come back after a restart
with a hole in it — the hours mirador was not running and observed nothing —
and it cannot mark that hole. Same discipline as weather showing a reading's
age rather than implying it is current.

**The day turning is recorded by the shell, not by a panel.** `App` holds the
date for this and no widget owns it. The todo panel notices a rollover too, but
a dashboard whose day-divider vanishes when you switch off the task list would
be strange — and it is the one source that always fires, which is what keeps the
panel from looking broken on a setup with no calendar and nothing falling due.

**The first read of a calendar is not news.** `AgendaPanel::known` is `None`
until the first successful read, so a startup does not announce every event in
your `.ics`. A log that opens with forty entries is a log nobody reads twice.

## A panel dialog is drawn by the shell, not the panel

`Panel::overlay` returns the open prompt and `App::render` draws it after every
panel, beside the help overlay and the picker. This is not tidiness: panels are
drawn in order, so anything a panel paints outside its own rectangle is painted
over by the panels after it. The zone picker was first written to draw itself
and came out interleaved with the task list, because the clock is panel one and
nine panels are drawn on top of it.

The corollary is that a panel's `render` must stay inside the rectangle it is
given. Nothing enforces it; the failure looks like a corrupt terminal.

## Recently settled, so it is not re-litigated

**The layout grid stays two levels** — `rows: Vec<Row{height, panels}>` — rather
than becoming recursive splits. Nested splits would make `[layout]` unreadable
and un-hand-editable, and would break the textual-edit approach the whole
persistence story rests on. The known cost is that panels of unlike natural
height in one row waste space; the mitigation is that arrange mode makes it
easy to group like with like.

**The default stays three rows.** Measured at 120x40 before deciding: nothing
truncates, all ten panels are legible, and the blank space in the clock is
inherent to a row sharing its height with weather rather than being waste to
reclaim. Arrange mode can open and close rows, so the row count is a gesture
now rather than a number shipped for everyone.

**`layout_edit` has two paths and the split matters.** Numbers-only changes are
still one-line edits, because rebuilding a row would reflow a hand-aligned
block on every `Ctrl+arrow` repeat. Structural changes rebuild the row from
captured panel entries — a panel's line *plus the comments above it*, looked up
across the whole block so a moved panel takes its explanation with it. Before
this, reordering within a row failed *silently*: the loop matched by name, saw
both panels still present, emitted nothing, and the round-trip check refused a
change the user had watched happen.

**Esc cancels in arrange mode and commits in the picker.** Inconsistent, and
deliberate: each picker change is one keystroke to undo, an arrangement is not.
The mode's legend names both keys, so there is nothing to guess.

## The road to 1.0.0

1.0 is not "everything is built" — that was true at 0.14.0. It is a promise, and
the promise has three parts: **your config keeps working, your data files keep
working, and there are no known crashes or hangs.** Everything below exists to
earn one of those three.

Ship 0.x releases freely along the way. A version number is cheap; the 1.0 one
is not, because it is the one people quote back at you.

### Phase 1 — finish the adversarial review

**`quote`: done, and clean.** No defects. Worth recording *why*, because the
reasons are load-bearing and easy to remove by accident:

- Numbers too large for an `f64` are refused by the JSON parser before any
  guard sees them. That is stronger than a downstream check and it is why no
  downstream check exists — so a future move away from `serde_json`'s strictness
  would need one.
- `change_pct` returns zero rather than infinity for a previous close of zero,
  and the sparkline returns its middle glyph for a series with no span. Both
  divisors are guarded at the point of division.
- Symbols reach the URL through a strict RFC 3986 unreserved allowlist, so
  nothing in a symbol can alter the request — while `.` passes through, which is
  what keeps `BRK.B` working.

All four market-data promises are enforced rather than merely written down, and
were re-checked: no key anywhere, `WatchlistFile` carries symbols and no prices,
`refresh_secs.max(60)`, and `stagger_ms.clamp(100, 10_000)`.

**`store` and `state`: done.** Three findings, all about data rather than
speed.

`write_atomic` used one `.tmp` name per *file*, not per *write*. Two mirador
windows — one per monitor — then raced for it: eight concurrent writers failed
2,100 of 2,400 saves, and a failed save is reported, so the second window filled
with complaints. Behind that sat a narrower hazard: `File::create` truncates, so
a second writer emptied the first's half-written temporary and the first would
have renamed *that* into place. Never reproduced, and it needs no reproducing to
be worth removing. Names are unique per write now.

A rename installs a new file, created per the umask, so `chmod 600` on a task
list was widened to 0644 on the next save. Permissions are carried across.

And invariant 17 was broken again, by a new route. `[agenda].file` ships
commented out, so `from_config` produced `None` while the panel reported a
*resolved* path — they differed, so one keystroke anywhere wrote the resolved
default into the state file, after which editing `[agenda].file` did nothing.
The rule is not only "compare against the config"; it is **compare against the
config in the same terms the panel will report**. A test now asserts that an
untouched default dashboard writes nothing at all.

**Per-frame allocation, swept across all twelve panels: no further offenders.**
Three fixes of the same shape in one day looked systemic, so it was measured
rather than argued about. It was not systemic; the agenda was an outlier.

The property that matters is not the size of the number, it is what the number
scales with. **A panel may allocate in proportion to what is on screen. It must
not allocate in proportion to how much data you have.** The agenda broke that —
a recurring calendar expands, and it cloned the expansion four times a frame —
and nothing else does. Verified by loading 5 items and then 500: `todo` stayed
at 667 allocations a frame both times, `notes` at 783 and 768.

For reference, allocations per frame with each panel alone, on an idle
dashboard. Roughly 150 of each figure is the shell — frames, layout, status bar
— so `cpu`, `network` and the fixed `agenda` are at the floor:

```
notes 1118   calendar 892   todo 885   weather 755   clocks 438
pomodoro 404  stocks 348    watchlog 271  news 252
agenda 175    cpu 174       network 159      (whole dashboard: 1345)
```

Nothing here is worth optimising. A thousand small allocations a frame at about
one frame a second is ordinary work for building styled spans, and the
measurement exists to catch *growth*, not to be minimised.

**To repeat the measurement:** put a counting `GlobalAlloc` in `main.rs`, wrap
the `terminal.draw` call in `App::run` to record the delta, and write it to a
file each frame. `unsafe_code = "forbid"` in Cargo.toml has to be relaxed for
that build, so do it on a scratch branch and check `git status` is clean
afterwards — shipping that lint relaxed would be a worse bug than any it found.
Then generate one config per widget with a single-panel `[layout]` and run each
for a few seconds.

**The fix to copy, if a panel ever does break the rule:** cache the shared state
in the panel when `tick` sees the generation move, and have `render`, `counter`
and `alert` read the cache. `agenda` and `news` both do this.

**`ical` and the agenda panel: done.** The parser itself held up — two
thousand recurring events in 32ms, `COUNT=999999999` clipped by the window,
fifty thousand folded lines and twenty thousand nested components all bounded.
The panel around it did not: `render`, `counter` and two key handlers each
cloned the whole expanded event list, at 210,000 clones per thirty idle seconds
against three hundred daily meetings. `counter` was the worst of them, being
called on every frame with nothing guarding it. All four now read a cache
filled once per tick.

Also bounded the `.ics` read at 10MB, matching what `ureq` already enforces on
the network side. Reading a calendar costs several times its size.

**`layout_edit` and `migrate`: done.** Found that both flattened CRLF to LF —
`str::lines()` strips the `\r`, so a Windows config was rewritten entirely
because one panel moved. `store::line_ending` now carries the file's own ending
through both. `migrate` had a third site the first fix missed, in a `writeln!`
that hard-codes `\n`; the test caught it, which is the argument for writing the
test before believing the fix.

Also added: eighty-eight structural mutations of the *real shipped config*,
asserting the only two acceptable outcomes — applied and exactly right, or
refused and untouched. "Wrote something plausible" is the outcome this module
exists to make impossible.

One pass covered the code written on 27 July and found a hang and two per-frame
allocation faults. The rest of the program has not had the same treatment.

Not yet reviewed: `layout_edit` (the structural path especially), `themes`,
`prompt`/`textfield`/`textarea`, `arrange`, `ical`, `quote`, `migrate`, `state`,
`store`, `chart`, `samples`, `selection`, `glyphs`.

*Exit:* every module reviewed, findings fixed or recorded with a reason.

### Phase 2 — harden the untrusted-input boundary

Five things consume input nobody here controls: `ical` (your `.ics`), `feed`
(third-party RSS), `layout_edit` and `themes` (hand-edited TOML), and
`grid::wrap` — which is where the hang was, fed by third-party headlines.

That is not a coincidence and it is the strongest signal from the review: the
one hang found was at exactly this boundary, in the one function that loops over
attacker-influenced text. Property tests over generated input are the cheap
version and want no new tooling; a fuzz target is the thorough one and wants
nightly.

*Exit:* no arbitrary input to any of the five produces a panic, a hang, or
unbounded memory.

### Phase 3 — soak

Nothing here has run for a day. This phase cannot be done by an agent in a
headless tmux and needs a person with a real terminal:

- **Terminal focus reporting** — never verified at all. The watch log's rule
  line falls back to the last keypress until somebody confirms it, and tmux
  needs `focus-events on`.
- **A real midnight** — the day-rollover entry has only met a test that winds
  the date back.
- **A full day idle**, watching memory and CPU.
- **Windows**, which has been started exactly once.

*Exit:* the list of unverified behaviour is empty.

### Phase 4 — freeze the formats and say so

1.0 means a config written today opens in 1.9. That needs an audit rather than
an assumption: every key in `assets/default_config.toml` reachable and
documented, every data file (`todos.toml`, notes, `watchlist.toml`,
`zones.toml`, `state.toml`, `update-check.toml`) either forward-compatible or
migrated, and `migrate.rs` covering every rename ever shipped.

*Exit:* a compatibility statement in the README that is true.

### Phase 5 — cut 1.0.0

When phases 1–4 are done, and not because a date arrived.

## Feature freeze

**New features are frozen ahead of 1.0.0.** Bug fixes, documentation and tests
only. Anything that adds a panel, a key or a config value waits — the point of
the freeze is to find out what the last round actually broke, and adding to it
defeats that.

Adversarial review found one hang and two per-frame allocation faults in code
written the same day, all of them in paths the tests exercised and the tests did
not catch. That is the argument for the freeze, not an argument against the
tests: the hang needed a two-cell glyph in a one-cell column, and nothing was
going to guess that from reading.

## Open work

Nothing. All three gaps named in the original product analysis are built — the
`.ics` agenda, the watch log, and the on-fire signal — along with named themes,
arrange mode, and in-panel editing for everything the UI can change.

That is a fact about the *list*, not a claim the program is finished. Two things
shipped recently have never run in the conditions they were built for, and both
are recorded where they live rather than here: terminal focus reporting for the
watch log's rule line could not be verified at all (a headless tmux has no
attached client to have focus), and the day-rollover entry has only ever been
exercised by a test that winds the date back. The first real midnight is the
first real test of it.

When something goes back on this list, put the *reason* beside it. Every entry
that was ever useful here said why it mattered; the one that got quietly dropped
and had to be added back was the one that did not.

## Housekeeping

- **`docs/demo.gif` is regenerated by `docs/record-demo.sh`**, which drives a
  real build under tmux, samples the screen, and hands an asciicast to
  [`agg`](https://github.com/asciinema/agg) — a single Rust binary, deliberately
  chosen over `vhs` because `brew install vhs` pulls in ffmpeg, Go and cmake for
  a job that needs none of them.

  ```sh
  cargo install --locked --git https://github.com/asciinema/agg
  ./docs/record-demo.sh
  ```

  Four things in that script are load-bearing and none of them are obvious:

  - **Build before redirecting `HOME`.** The recording uses a throwaway home so
    it shows a genuine first run, but rustup keeps its toolchain config under
    the real one and a cargo that cannot find it refuses to pick a compiler.
  - **Normalise the cast so the first event is at t=0.** Otherwise the first
    capture lands a few milliseconds in, `agg` faithfully renders those
    milliseconds of empty terminal, and the GIF opens on a black frame — the one
    a still preview shows, and a flicker on every loop.
  - **150x42, not smaller.** The clock drops its block numerals when its row is
    short, and those numerals are the one thing in mirador that looks like
    nothing else.
  - **The order of the segments is now taste, not a workaround.** It used to
    matter: toggling any panel rebuilt every panel, so starting the pomodoro
    before the picker segment recorded the timer silently resetting, and the
    weather and stocks panels spent a fetch cycle showing "loading" afterwards.
    `rebuild_panels` carries panels across now, so neither happens.

- **The README's images float; its prose does not.** `docs/demo.gif` is
  referenced by absolute `raw.githubusercontent.com/.../main/...` URL, because
  `Cargo.toml` excludes `/docs`, `*.gif` and `*.png`, and a relative path would
  render on GitHub and 404 on crates.io. The consequence is easy to miss:
  replacing the capture updates it on *every* published version at once, while
  the caption and alt text around it stay frozen at whatever each release baked
  in. 0.3.0 spent an hour showing the pomodoro panel above a caption explaining
  that the shot predated it. A caption that describes the picture has to ship in
  the same release as the picture.

  **`docs/screenshot.png` is no longer in the README and must not be deleted.**
  0.1.0 through 0.5.1 all shipped a README pointing at that URL, and those
  READMEs are frozen on crates.io. Removing the file would break the image on
  six published pages that nobody can edit. It costs nothing to keep — `/docs`
  never reaches the crate — so it stays as an asset the past still needs. The
  same will be true of `demo.gif` the day something replaces it.

- Originally built in a Linux container, where `sysinfo`'s macOS CPU and network
  paths went unexercised. Both have since been run on macOS against a real
  terminal under `tmux` and report sensible figures. Windows has since been run
  too — see the platform note below.
- **`0.7.1` is released**, on crates.io and as a GitHub release with binaries
  for macOS arm64, macOS x86-64, Linux x86-64 and Windows x86-64. `0.0.0` is still on
  crates.io below it — the name reservation that went out first, since
  reservation is first-come with no reclamation.

  Cutting a release is a tag push and a `cargo publish`, in that order. Bump
  the manifest, commit, *then* tag: a tag that disagrees with `version` in
  Cargo.toml is refused. The workflow does **not** publish to crates.io — that
  step stays manual and comes after the tag.

  **`main` is protected, so "commit" there means *merged*, not committed
  locally.** The version bump reaches `main` through a PR like anything else,
  and the tag belongs on the squashed commit that lands. Tagging the local
  commit first appears to work — the tag pushes, the workflow runs, the
  artifacts are correct — and then the squash orphans the commit underneath
  it, so `git describe` on `main` cannot see the release. 0.8.0 is tagged that
  way and was deliberately left alone: the trees are byte-identical, and
  force-moving a tag re-triggers the release workflow against a release that
  already exists, which is a worse problem than the wart it fixes. Merge
  first, then tag.

  **Every artifact carries a GitHub provenance attestation**, via
  `github-attestations = true`. Verified working at 0.5.0 for both an archive
  and a `-update` binary, both tracing to
  `release.yml@refs/tags/v0.5.0`:

  ```sh
  gh attestation verify --repo jchultarsky/mirador mirador-aarch64-apple-darwin.tar.gz
  ```

  Two traps when checking it. `gh attestation verify` prints its success banner
  only to a TTY, so redirecting it gives an empty file that looks like a
  failure; and `$?` after a pipe reports the *pipe's* exit status, not `gh`'s.
  Confirm the check is real by pointing it at a repo that did not build the
  artifact — that must exit 1 with a 404. Nothing else is manual — targets, checksums,
  installers and release notes are all the workflow's job.

- **`.github/workflows/release.yml` is generated by `dist` and must not be
  hand-edited.** It is build output. Change `[workspace.metadata.dist]` in
  Cargo.toml and run `dist generate`; the workflow also runs `dist plan` on
  every pull request, so drift surfaces there.

  ```sh
  cargo install cargo-dist --locked   # the binary is `dist`
  dist plan                           # what a release would produce
  dist plan --tag=v0.1.0              # what that exact tag would produce
  dist generate                       # rewrite release.yml after a config change
  ```

  `dist init` will try to set `[profile.dist] lto = "thin"`, which quietly
  undoes the `lto = true` in `[profile.release]`. That override is deliberately
  removed — every release so far has been fat-LTO'd and there is no reason for
  the binaries to get slower. If you re-run `init`, check it has not come back.

  `dist` provides its own version-tag guard (`dist plan --tag=v9.9.9` exits
  255) and its own prerelease detection from the semver hyphen, which is why
  the hand-written versions of both were dropped when it took the file over.

- **Windows runs**, confirmed by the owner on 25 July 2026 — so all three
  shipped targets have now been started, not merely built. Treat it as the
  least-travelled of the three rather than as unknown: macOS and Linux are used
  daily and Windows has been checked once.

  That run also came through the **PowerShell installer** (`irm … | iex`) in
  the default Windows terminal, which is the first real-world exercise of that
  path — everything else about the installers had only been checked from macOS.
  `mirador-update` is installed alongside by the same installer but has not been
  run on Windows; the macOS one has.

  `aarch64-pc-windows-msvc` is deliberately absent because `ring`, reached
  through `ureq`'s TLS, does not build for it. musl is absent for the same
  C-toolchain reason. `ring` is also why the Windows target cannot be
  cross-checked from macOS — `cargo check --target x86_64-pc-windows-msvc`
  fails locally on `cc`, and that is the host missing MSVC rather than anything
  wrong with the code.

- macOS notarization is unsupported by `dist` and does not matter here:
  Gatekeeper's quarantine flag comes from browsers, not `curl`, so
  shell-installer users never see a prompt. Not worth $99/yr for a TUI.

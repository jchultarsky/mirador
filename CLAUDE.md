# CLAUDE.md — working notes for mirador

Context for anyone (human or agent) picking this repo up cold. Design rationale
that belongs to users lives in `README.md` and `CONTRIBUTING.md`; this file is
the stuff you would otherwise have to reverse-engineer from the diff.

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
cargo test                                    # 330 tests, all fast, no network
cargo clippy --all-targets -- -D warnings     # must be silent
cargo fmt --all -- --check                    # must be silent
cargo run -- --print-config > /tmp/m.toml     # scratch config to experiment on
cargo run -- --config /tmp/m.toml
```

**The bar is zero warnings and zero errors, with fmt and clippy clean.** The
crate enables `clippy::pedantic`. When a lint is genuinely wrong, add a targeted
`#[allow]` *with a comment saying why* — do not widen the allow list in
`Cargo.toml`.

To eyeball the rendering without a terminal:

```sh
cargo test dump_dashboard -- --ignored --nocapture   # renders to stdout
```

To drive it in a real terminal headlessly, run it under `tmux` and
`tmux capture-pane -p`. Several layout bugs only showed up this way — the
`TestBackend` dump will not catch a panel whose content is pushed off the
bottom, because nothing errors.

## Architecture

```
main.rs      CLI parsing, terminal setup
app.rs       event loop, focus ring, grid geometry, help overlay, status bar
panel.rs     the Panel trait — the only extension seam
frame.rs     panel frames, Binding type, key hints punched into borders
grid.rs      shared column grid with named headers
chart.rs     braille graphs + baked colour gradients
glyphs.rs    block numerals, bold-uppercase labels, weather art
theme.rs     colours and gradient stops
config.rs    TOML config, validation, default file
migrate.rs   textual in-place upgrade of configs written by older versions
task.rs      task model + atomic TOML store
note.rs      note model + atomic TOML store
quote.rs     Quote + the pluggable QuoteSource trait + the watchlist store
textfield.rs single-line text editor used by task entry
textarea.rs  multi-line text editor used by note bodies
dateinput.rs due-date entry
widgets/     clocks, weather, todo, notes, stocks, calendar, cpu, network
```

`Panel` has two input hooks. `handle_key` goes to the *focused* panel;
`handle_mouse` goes to the panel under the *pointer*, which is deliberately not
the same thing — a scroll wheel must move the list it is aimed at without
yanking the keyboard away from what the user was typing in.

Adding a widget: implement `Panel`, add a config struct, add the name to
`WIDGET_NAMES` and an arm to `build()` in `widgets/mod.rs`, document it in
`assets/default_config.toml` and the README. Nothing else needs to know.

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

## Theming — decided, not yet built

Follow Helix. `theme = "name"` plus theme files in
`<config>/mirador/themes/<name>.toml`, with:

1. **Dotted-scope fallback** (`ui.border.focused` → `ui.border` → `ui`) via
   `iter::successors`. ~15 lines, makes a two-line theme valid, and makes
   `border_focused` stop being a special case.
2. **`[palette]`** with `"default"` → `Color::Reset` and the 16 ANSI names
   pre-seeded. Literal colour names live *only* in the palette; every other key
   is semantic.
3. **`inherits`** with cycle detection and depth-2 palette merge.
4. Full style objects (fg/bg/modifiers).

Ship bundled: `ansi` (ANSI names only, safest default), `default` (true-colour
dark, `inherits = "ansi"`), `default-light`, `high-contrast`. Nobody downsamples
colour — the universal strategy is a separate low-colour theme picked on
`COLORTERM`. Keep a compat shim for one release (TOML type distinguishes an
inline `[theme]` table from a `theme = "name"` string).

## Open work, in priority order

1. **Settings editable from the UI**, starting with `[weather].location`.
   Blocked on a decision: mirador deliberately *never rewrites the config*, so
   either that stance changes or edited settings go to a separate state file.
   The watchlist took the second route and it worked well — that is the
   precedent, but applying it to *all* settings is still the owner's call.
2. Calendar panel reading a local `.ics` — the shipped `calendar` widget is a
   date grid only, deliberately offline; events are a separate, larger panel.
3. Theme system per above.
4. "What changed since I last looked" markers.
5. **`task.rs` reuses ids after a deletion** (`max(id) + 1`). `note.rs` hit the
   same bug and now keeps a high-water mark; port that fix across.

## Housekeeping

- Originally built in a Linux container, where `sysinfo`'s macOS CPU and network
  paths went unexercised. Both have since been run on macOS against a real
  terminal under `tmux` and report sensible figures. Windows remains untested.
- **The crates.io name is not yet reserved.** Publish a `0.0.0` placeholder;
  reservation is first-come and there is no reclamation.

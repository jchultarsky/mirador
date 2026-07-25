# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Stock watchlist panel: last price, the day's change in currency and percent,
  and an intraday sparkline. `a` adds a symbol, `d` removes one, `r` refreshes.
  The watchlist is stored as a data file rather than in the config, which is
  what lets the panel edit it — mirador deliberately never rewrites the config,
  so a watchlist living there could only ever be changed in an editor. Config
  seeds the first run and nothing after it.

  The quote source is a trait from the first commit rather than a later
  refactor: the default gates on IP reputation instead of on a key and blocks
  datacenter and VPN ranges outright, so the same build works on a laptop and
  fails on a VPS. That can only be routed around by swapping the source. No API
  key is bundled, polling is clamped to once a minute, symbols are fetched one
  at a time rather than in a burst, and prices are never written to disk.
- Notes panel: free-form notes with a title, a body and the dates they were
  written and last edited. Master-detail, the shape a mail client uses — the
  list of titles and dates sits beside the selected note's body, because a
  note's whole value is the text inside it and making the reader press a key to
  see any of it turns "glance at the dashboard" into "operate the dashboard".
  The split follows the panel: side by side when there is width for both,
  stacked when there is not. `/` searches bodies as well as titles, since the
  title you wrote in a hurry is often not what you later search for. Notes are
  stored as hand-editable TOML and written atomically, like tasks.
- Calendar panel: month grids in the shape `cal` prints, showing the current
  month and the next, with today marked in reverse video. `n`/`p` step a month,
  the arrows step a month or a year, `t` returns to today, and the wheel steps
  a month. Months lay out side by side when the panel is wide and stack when it
  is tall; a month block is always six week-rows high, so scrolling does not
  make the panel jump.
- Mouse support, off the `[general].mouse` switch. Clicking a panel focuses it;
  scrolling the wheel moves the list or calendar under the pointer *without*
  taking focus, so the wheel cannot steal the keyboard from what you were
  doing. A panel in a text-entry state vetoes mouse actions exactly as it
  vetoes global keys, so a stray click cannot strand a half-typed task.
- Panels resize from the keyboard, the way tmux panes do: `Ctrl+←/→` trades
  width with the neighbouring panel in the row, `Ctrl+↑/↓` trades height with
  the neighbouring row. The total is held constant so widening one panel
  narrows exactly one other and nothing else on screen reflows, and no panel
  can be squeezed to nothing — a panel with no width could never be focused to
  get its space back. Resizes last for the session; the config is not rewritten.
- The dashboard now redraws only when something changed, rather than once per
  event loop pass. Mouse reporting made this necessary — terminals send an
  event for every cell the pointer crosses — but it also means an idle
  dashboard left open all day costs a redraw a second instead of four.

- Column grid with named headers, shared by every panel that lists things.
  Tabular data now reads as a table: fixed column positions, right-aligned
  numbers and dates, and headers in the letterspaced utility face. Optional
  columns drop whole rather than squeezing when a panel is narrow.
- Braille history graphs at 2 samples per cell and 4 levels per row, with the
  colour gradient running vertically by magnitude. The profile is static as
  data scrolls, so a graph left on screen all day does not shimmer.
- Three-stop colour gradients, baked to a lookup table at startup and shared
  between a panel's graph, meter and numeric readout.
- Key hints drawn into the focused panel's bottom border, the panel's jump key
  in its title, and a status counter in the top-right — all costing zero
  interior rows. Hints are shown only for the focused panel.
- `Binding` type: one declaration feeds the border hint, the status bar and the
  help overlay, so hints cannot drift from the keys they describe.
- Scalable block numerals for the clock, sized to the panel. Seconds ride small
  beside the large `HH:MM` when the full time will not fit.
- Weather art for ten sky conditions.

### Changed

- Labels are bold uppercase rather than letterspaced: `NEXT HOURS`, not
  `N E X T  H O U R S`. Tracking was meant to read as an engraved instrument
  label and instead read as stretched text, and it more than doubled the width
  of every label — which is why the grid header carried logic to demote a whole
  row to plain caps when one column could not fit. That logic is gone with it.
- The weather forecast is hourly rather than daily, and every row is labelled
  with the hour it applies to.
- The clocks panel renders the first configured zone large, with the rest as a
  labelled table showing offsets *relative to that zone* rather than to UTC.
- Panels have one column of interior padding.
- Focus is signalled by dimming unfocused panels rather than brightening the
  focused one, so exactly one thing on screen is at full brightness.
- The default palette is brass, verdigris and slate rather than cyan. Body text
  is `reset`, inheriting the terminal's own foreground.
- Seconds are shown in the clock by default.

### Fixed

- `Enter` on a task opens it for editing rather than marking it done. Enter
  means "go inside" everywhere else, so binding the most reflexive key in the
  list to a state change on the highlighted row was a trap. Completing a task
  is `space`, which reads as a checkbox.
- The task panel's key list now matches the keys it actually handles. `Enter`,
  `n`, the arrow keys, `Home`/`End`, `PgUp`/`PgDn` and `Esc` all worked but
  appeared nowhere in the help, so the only way to find them was to read the
  source. A test now pairs every handled key with the binding that documents
  it, and fails if either side gains an entry the other lacks.
- Unknown keys in the config file are now rejected at startup instead of being
  silently ignored. A config written by an older version used to load with its
  stale keys quietly dropped, which made current code look like a stale build.
  Keys renamed since 0.1.0 name their replacement in the error.
- `--migrate-config` updates a config written by an older version in place,
  keeping a `.bak` of the original. The migration is textual, so comments and
  formatting survive; it refuses to write anything that would not load.
- Text in tables is measured in terminal cells rather than characters. Glyphs
  like `☀` occupy two cells, so counting characters shifted every column after
  them and put values under the wrong headers.
- Weather marks use only emoji whose measured width matches what terminals
  draw; several obvious ones (rain cloud, snow cloud, cloud, fog) report one
  cell and render two. A test asserts this.
- Forecast cells always show a value: `0%` rather than a blank, which reads as
  a broken column.
- Task rows no longer shift horizontally when a task has no due date.
- Key hints are no longer duplicated between the panel body and its frame.

## [0.1.0] - 2026-07-25

Initial release.

### Added

- Terminal dashboard with a configurable grid layout. Rows and panels use
  relative weights, so a layout keeps its proportions at any terminal size.
- `Panel` trait as the extension seam; each widget owns its own state and
  refresh cadence, and the application shell handles only layout, focus and
  event dispatch.
- **Clocks** widget: any number of world clocks by IANA timezone, with the
  literal `local` for the system zone. Configurable time and date formats, and
  optional UTC offsets. Unresolvable zones are reported inline rather than
  dropped.
- **Weather** widget: current conditions plus a 1–7 day forecast from
  Open-Meteo, which needs no API key or account. Location is geocoded from a
  place name, or given directly as latitude and longitude. All network I/O runs
  on a background thread, so a slow request never blocks rendering.
- **To-do** widget with full create, read, update and delete:
  - Tasks carry a title, notes, due date, priority, tags and completion state.
  - Add and edit through an in-panel form; delete asks for confirmation.
  - Due-date input accepts ISO dates, `today`/`tomorrow`, weekday names, and
    offsets such as `+3d` or `2w`. Unrecognised input is rejected with an
    explanation.
  - Five sort modes, including a "smart" default that surfaces overdue work
    ahead of high-priority work that is not yet due.
  - Live filtering across titles, tags and notes.
  - Storage is a plain, hand-editable TOML file. Writes are atomic, and save
    failures surface in the panel rather than being swallowed.
- **CPU** widget: average utilisation as a scrolling chart, with per-core
  meters and configurable warning and critical thresholds.
- **Network** widget: receive and transmit throughput as scrolling charts, with
  session totals. Rates are derived from real elapsed time between samples, so
  a delayed tick reports the correct rate rather than an inflated one.
- Configuration in a single TOML file, written with full comments on first run.
  Unknown widget names, malformed colours and invalid units are rejected at
  startup with actionable messages.
- Command-line flags: `--config`, `--config-path`, `--print-config`, `--help`
  and `--version`.
- Help overlay bound to `?`, listing global bindings and those of the focused
  panel.

[Unreleased]: https://github.com/jchultarsky/mirador/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jchultarsky/mirador/releases/tag/v0.1.0

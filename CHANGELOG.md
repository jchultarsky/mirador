# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.9.1] - 2026-07-27

### Changed

- **Arrange mode says that rows can be opened.** The legend read
  `↑↓ move rows`, which a reader reasonably took to mean "move between the rows
  you have" — and then asked how to make a new one, which the mode had been
  able to do all along. It now reads `↑↓ at the edge opens a new row`, paid for
  by collapsing the two movement hints into one: which arrow goes which way is
  obvious the moment you press it, because the panels move.

  The legend also drops hints whole rather than clipping them when the terminal
  is narrow, keeping `Enter keep` and `Esc cancel` longest. Half a hint reads as
  a rendering fault; a missing one reads as a narrow terminal, which is what it
  is.

- The README now answers "how do I manage rows" directly instead of leaving it
  inside a sentence about moving panels.

## [0.9.0] - 2026-07-27

### Added

- **Arrange mode.** Press `m`, then move the focused panel with the arrows.
  `←`/`→` swap it with its neighbour; `↑`/`↓` move it between rows, landing it
  under wherever it already was rather than at the same index. Push it past the
  top or bottom edge and it takes a row of its own — which is how you get a
  fourth row without opening an editor — and the last panel out of a row closes
  that row. The real panels move as you press, `Enter` keeps it and `Esc` puts
  everything back. A panel keeps the width you gave it, and the row weights
  always add up to what they added up to before.

- **`Ctrl+arrow` resize is discoverable.** It has worked since long before this
  and only ever appeared in the `?` overlay. It is in the status bar now, and
  arrange mode's legend names it.

- **Panels can ask for a setting.** The agenda takes an `.ics` path with `f`,
  the weather takes a location with `L`, and the clock adds a zone with `a` and
  removes one with `d`. `Tab` completes — filesystem paths for the agenda,
  timezone names for the clock — as far as every candidate agrees. Each panel
  checks what it can before accepting: the agenda stats the file, the clock
  resolves the zone, and a refusal keeps what you typed so one wrong character
  does not cost you the whole path.

  Weather location was the oldest item on the open-work list.

- **World clocks are a data file.** `zones.toml` beside your tasks, the same
  arrangement as the watchlist and for the same reason: mirador never rewrites
  your config, so a list kept there could only be changed in an editor.
  `[clocks].zones` seeds your first run and is not read again.

### Fixed

- **A narrow panel no longer eats the bracket that closes its title.** At around
  100 columns the frame drew `╭┤9 CPU┤18 cores├╮` — the title and the counter
  are separate border segments and the title was being clipped, taking its `├`
  with it, which reads as a broken frame rather than a narrow one. The title is
  shortened instead, and below about 14 columns it is dropped rather than drawn
  as an empty `┤├`.

- **Reordering panels in the config actually works.** `layout_edit` matched
  panels by name and had no concept of order, so moving a panel along its row
  produced no edit at all and the safety check then refused a change that had
  visibly happened on screen. It could not open or close rows either. It can do
  all three now, and a moved panel takes the comments written above it along
  rather than leaving them to caption whatever slid into its place.

## [0.8.0] - 2026-07-27

### Added

- **An optional update check.** `[general].check_for_updates = true` asks
  crates.io, at most once a day, whether a newer mirador exists, and says so
  once in the status bar if it does.

  **Off by default and staying that way.** Everything else in mirador reaches
  the network only because a panel you placed needs data; this would reach it on
  its own behalf, telling a third party your IP address and that you run this
  program on a schedule you did not pick. Small, and still the thing "nothing
  phones home" was promising — so it is yours to turn on.

  When on: no identifier is sent, the answer is cached in `update-check.toml`
  beside your tasks so a day of restarts costs one request, a failure is silent,
  and the notice retires on your first keypress. `NO_UPDATE_CHECK=1` or
  `DO_NOT_TRACK=1` in the environment overrides the config.

- **The README now says how mirador is built.** It is written with heavy AI
  assistance, and anyone weighing up the code deserves to know that without
  having to infer it from the commit history.

## [0.7.1] - 2026-07-27

### Fixed

- The two links to dist's documentation pointed at `opensource.axo.dev`, which
  no longer resolves. They point at `axodotdev.github.io` instead.

## [0.7.0] - 2026-07-27

### Added

- **An agenda panel**, reading a local `.ics` file. The gap it fills was named
  early and left open for a long time: tasks are self-paced and a meeting is
  not, so a dashboard that could answer four questions and none of them was
  "you are in a call in ten minutes" was missing the one with a deadline.

  It is deliberately offline. mirador does not sign in to a calendar server —
  that means an account, a token to refresh, and a background process holding
  your credentials. Point `[agenda].file` at a calendar you already have and
  keep it current however you like; it is re-read on a timer and on `r`.

  Recurring events are expanded for the rules people actually have — daily,
  weekly with `BYDAY`, monthly, yearly, with `INTERVAL`, `COUNT`, `UNTIL` and
  `EXDATE`. Anything more elaborate shows only its first occurrence rather than
  guessing: a calendar that invents a meeting costs a wasted trip, where one
  that misses a repeat costs a glance at the real thing.

  Parsed in-tree with `jiff`, which mirador already has. `icalendar` and `rrule`
  would have added 30 crates and **2.3 MB** to a 3.4 MB binary — measured — most
  of it `chrono-tz` carrying a second timezone database.

  An unconfigured panel says "No agenda file" and how to set one, in ordinary
  colours — that is a panel nobody has set up, not a fault. A file that exists
  and cannot be read is the fault, and says so.

  **The default layout now places ten panels**, so the last of them has no
  number to jump to; `1`–`9` covers the rest and `Tab` reaches everything. The
  bottom row is reordered so the pomodoro keeps the width its numerals need.

## [0.6.0] - 2026-07-27

### Changed

- **`[cpu].sample_secs` and `[network].sample_secs` now default to 2.** Since
  the redraw follows visible change, every sample is a new number and every new
  number costs a repaint, so these two panels set the floor on what an idle
  dashboard does. Measured at 400x100 on the default layout, redraws per idle
  minute:

  |                        | `sample_secs = 1` | `= 2` |
  | ---                    | ---               | ---   |
  | `show_seconds = true`  | 95                | 81    |
  | `show_seconds = false` | 66                | 36    |

  The second row is the change. The first is what the clock costs: with seconds
  on it asks for a repaint every second and swamps the rest, which is worth
  knowing before blaming the graphs.

  The charts now cover twice the wall-clock time for the same buffer, and the
  span beside the figure says so. The cost is a two-second average, so a brief
  spike reads slightly lower — set it back to 1 if you would rather watch
  closely than leave it open all day.

  **Only new installs are affected.** The config seeds on first run and mirador
  never rewrites it, so an existing `config.toml` keeps whatever it already
  says.

## [0.5.2] - 2026-07-27

Documentation only; no code changed since 0.5.1.

### Changed

- The README opens on the demo recording alone. It previously showed a static
  screenshot of the same dashboard immediately above it, which made the same
  first impression twice; the recording is that screen plus what happens when
  you press something.

## [0.5.1] - 2026-07-26

### Fixed

- **Switching a panel on or off no longer disturbs the others.** Toggling
  anything in the `w` picker used to rebuild every panel, which reset a running
  pomodoro to 25:00 and sent the weather and market panels back to "loading" for
  a fetch cycle. Panels that are still placed are now carried across untouched.
  Keyboard focus follows its panel to wherever the new layout puts it, instead
  of staying on an index that may now be a different panel.

### Added

- A [demo recording](https://github.com/jchultarsky/mirador#readme) in the
  README, and `docs/record-demo.sh` that regenerates it from a real build.

## [0.5.0] - 2026-07-26

Fifteen items from an adversarial review of the whole project, worked in order
of how much each could cost someone.

**Upgrading:** two changes can stop a config that used to start. A misspelled
key under `[theme]` is now an error rather than being silently ignored, and an
absurd `[weather].refresh_minutes` or `[stocks].refresh_secs` is rejected rather
than wrapping. Both report the key and how to fix it. If your config was written
before 0.1.0, `mirador --migrate-config` will update it.

### Fixed

- **`Esc` no longer quits the dashboard.** It was in the global quit arm, so the
  reflex that closes a dialog closed the program — and any unsaved note went
  with it.
- **A long note can be scrolled to its end.** The body scrolled against its
  unwrapped line count, so wrapping hid the tail.
- **`Ctrl+S` and `Enter` save from either field of a form**, not just the one
  the cursor happened to be in.
- **Removing a symbol from the watchlist reports a failed save** instead of
  dropping the error.
- **The weather and stocks pollers stop when their panel goes away.** Neither
  loop had an exit, and the panel picker rebuilds every panel on every toggle,
  so a few passes over it left several threads polling the same endpoints —
  multiplying a request rate that `CLAUDE.md` claimed was enforced in code.
  Measured: 3 threads at rest, 3 after ten toggles.
- **mirador no longer panics at startup on Windows** on a machine up for less
  than a day. `Instant` there counts from boot, so back-dating one by 24 hours
  returned `None` and the `unwrap` fired before the terminal existed.
- **A preference can be un-set again.** Switching temperature units to metric
  and back left the state file insisting on metric, permanently. The comparison
  that decides what to record now happens once, against the config as it was
  read, rather than in each panel against the value it was built with.
- **`?` scrolls.** With the tasks panel focused on an 80x24 terminal the overlay
  had 29 rows of content and 22 rows to draw it in, so nine of that panel's
  seventeen key bindings — including `/`, `e`, `p`, `s` and `c` — were invisible,
  with nothing on screen to say so. Arrow keys, `PgUp`/`PgDn`, `Home` and `End`
  scroll it, and the footer shows where you are. On a terminal with room to spare
  nothing changes: any key still closes it.
- **A long task title no longer draws over the panel border.** Titles were
  truncated by counting characters against a budget measured in terminal cells,
  so every CJK character or emoji overflowed by one cell.
- **A misspelled key under `[theme]` is now reported** rather than accepted and
  ignored. This also un-blocked the `--migrate-config` hint for the pre-0.1.0
  `rx` and `tx` theme keys, which could never fire because those keys parsed
  cleanly.
- **The weather panel recovers from a failure to look up your location.** It used
  to end its fetch thread, then go on offering "r to retry" with nothing left to
  act on the key — most often after a laptop resumed before its Wi-Fi did.
- **A failed price fetch keeps the last price and labels it with its age.** One
  timed-out request used to blank the price, the change, the percentage and the
  sparkline together for a whole refresh interval; a fetch thread that stopped
  quietly showed confident numbers indefinitely. A retained price is now shown
  muted, with how old it is.
- Panel rectangles are indexed consistently, so a layout entry that builds no
  panel cannot shift every later panel onto its neighbour's rectangle — which is
  what mouse clicks are matched against.
- **A wide terminal no longer parks the whole row on one panel.** Past the point
  where a row's panels have all reached their useful maximum, the surplus was
  handed to whichever panel happened to be uncapped last. At 400 columns that
  gave the clock 302 of them — about 145 of which were blank, since its numerals
  stop growing at 158 — while the weather panel beside it sat at 51. The excess
  is now shared across the row.
- **The network panel's `SESSION` totals are the session.** They were the
  machine's since-boot counters, so on a laptop up three weeks the panel read
  128 GB within a minute of launch.
- **Holding `r` on the watchlist can no longer outrun the one-minute floor.**
  The limit was applied to the polling interval, and `r` bypassed the wait
  entirely — 8 requests in 2 seconds against a source that blocks by IP address.
- An absurd `refresh_minutes` or `refresh_secs` is rejected instead of wrapping
  into a tight loop against a free API.
- Panel width changes made with `Ctrl+arrow` are saved shortly after you stop,
  rather than only on a clean exit — closing the terminal window used to lose
  them.

### Changed

- **mirador redraws when something changes, rather than when a timer fires.**
  Measured on the default layout at 400x100: **243 redraws a minute before, 62
  after** — and before, the number was the same whether `show_seconds` was on or
  off, so turning seconds off changed what was on screen and not what the
  program ran. Battery life on a dashboard you leave open all day is the point.
- Every file mirador writes — including your config — now goes through one
  atomic write that flushes to the disk before replacing the original. The
  config was previously overwritten in place.
- The README leads with what mirador is for rather than with a feature
  comparison, and says plainly what using Yahoo's undocumented chart endpoint
  does and does not mean for you.

### Security

- Release artifacts carry GitHub build provenance attestations, and the shipped
  binaries embed their dependency list so `cargo audit bin` can check them.
- `main` requires all eight CI jobs, including Windows and a `cargo-deny` supply
  chain check. CI actions are pinned by commit rather than by tag.
- Documented honestly what the one-line installers verify — the shell one checks
  the archive's sha256, the PowerShell one checks nothing, and neither checks
  the updater. See [SECURITY.md](SECURITY.md#verifying-a-download).
- Private vulnerability reporting is enabled, which the security policy had been
  pointing people at while it was switched off.

## [0.4.0] - 2026-07-26

### Added

- **A panel picker.** `w` opens a dialog listing every widget with whether it is
  on; `space` toggles, and the panel appears or disappears immediately.
  Switching a widget on used to mean finding your config file and editing
  `[layout]` by hand — which is what the first person to meet the pomodoro
  panel actually had to do, after pressing `?` and not finding the answer there
  either. The status bar notice and the help overlay both name the key now.

  Panel sizes are remembered too: `Ctrl+arrow` no longer lasts only for the
  session.

### Changed

- **Layout changes are written back into your config**, reversing an earlier
  decision to keep them out of it. The rule was never really "do not write to
  the config" — it was "do not *reserialise* it", because a round trip through
  `toml` discards every comment in the file, including the ones mirador wrote to
  explain its own options. `--migrate-config` had already established the
  alternative: edit the lines that need editing and leave the rest alone.

  So `[layout]` is edited surgically. Adding a panel is a one-line diff; a width
  change rewrites one number and keeps its column alignment; comments inside the
  layout block survive.

  The safety property is a check rather than care: the edited text is parsed and
  compared against the layout that was asked for, and a mismatch throws the edit
  away. An unusually formatted config fails as "that did not stick", said in the
  picker, rather than as a broken file.

  Preferences that are not layout stay in the state file. The split is by what
  the setting *is*: `[layout]` is the part of the config people read and curate,
  so a change made in the UI has to show up there, while nobody keeps their
  preferred sort order under version control.
- Windows is described as working rather than as untried. The binary shipped in
  0.2.0 was built and packaged but had never been started on the platform, and
  the docs said so; it has now been run and works, installed with the PowerShell
  one-liner in the default Windows terminal. All three shipped targets have been
  started rather than merely compiled, and that is also the first real-world use
  of the PowerShell installer — the rest had only been checked from macOS. It is
  still the least-travelled of the three, which the README says instead of
  claiming a parity nobody has earned.

## [0.3.1] - 2026-07-25

### Fixed

- The README on crates.io no longer contradicts its own screenshot. The image
  is referenced by absolute URL so that it resolves on crates.io as well as
  GitHub — which means it tracks `main` and updated the moment a new capture
  landed, while the caption around it stayed frozen at whatever the last
  publish baked in. The published page ended up showing the pomodoro panel
  above a caption explaining that the shot predates it. The alt text was stale
  for the same reason, which matters more, since a screen reader has only that.

  Worth knowing for any future image: a floating URL and fixed prose drift
  apart by design, so a caption describing the picture has to ship in the same
  release as the picture.

## [0.3.0] - 2026-07-25

### Added

- `mirador-update`, installed beside the binary by the shell and PowerShell
  installers. It asks GitHub for the newest release and installs it if that is
  newer, so upgrading no longer means going back to the releases page. It runs
  only when invoked: mirador itself never checks for updates and does not know
  the program exists, which keeps the "no telemetry" line in the README exactly
  as true as it was. Anyone who installed with `cargo install` or from source
  upgrades the way they installed.
- **Settings changed from the keyboard are remembered across restarts.**
  Weather units, the task sort order, whether completed tasks show, seconds on
  the clock, and the pomodoro durations. The watchlist already did this for
  symbols; this is the same answer generalised.

  mirador still never rewrites your config — the property that makes it safe to
  hand-edit and keep in git. The config *seeds* a setting and a small state
  file beside your tasks records where you moved it since. Deleting that file
  puts everything back, which the file's own header says, because a preference
  you cannot remember setting needs an obvious way out.

  **Only what you actually changed is written.** Pressing `u` records the units
  and nothing else. The first version reported every panel's current value,
  which pins the config's own settings into the state file the moment any one
  preference moves — after which editing the config silently stops working.
  That passed its tests and was caught by running it.

  Panel sizes are deliberately excluded: `Ctrl+arrow` resizing is geometry
  rather than preference, and remembering it would mean a saved width quietly
  overruling a `[layout]` edited by hand.
- **Pomodoro panel.** A focus timer in the same block numerals the clock uses:
  `space` starts and pauses, `n` skips a phase, `r` restores the current one,
  and `+`/`-` change the length of the phase you are in. Focus is brass and
  breaks are moss, with the phase named above the numerals as well, because
  colour alone is a poor way to tell someone whether they are meant to be
  working. A paused timer greys out rather than blinking — this is a dashboard
  you leave open, and a flashing clock is the opposite of that.

  `+` and `-` move the phase length and the time left together, so adding a
  minute part-way through does not rewind you to the start. A phase that ends
  while you were away advances exactly one step against a fresh clock rather
  than chaining from the deadline it missed, so a laptop resumed after an hour
  does not race through six phases catching up.

  An optional chime, **off by default**, rings when a phase ends. With no
  command configured it is the terminal bell, which lets your terminal decide
  whether that means a sound, a flash, or nothing. mirador ships no audio
  library on purpose: playing a file means linking the platform audio stack,
  and on Linux that is a C library and its dev headers on every builder — a
  large amount of machinery for one notification. `chime_command` names a
  player instead, run directly with no shell in the way, and a player that
  cannot start is reported in the panel rather than silently doing nothing.

### Fixed

- The pomodoro panel stops claiming space it cannot use. It declared a maximum
  of 102 columns and 21 rows — the width of the numerals at the largest scale
  the clock allows — and on a wide terminal it took them, from the task list.
  The numerals are now capped at the scale where `MM:SS` is 38 columns by 5
  rows, which is already a chunky readout; scale 2 is 68 columns and scale 3 is
  98, and a timer occupying half a dashboard reads as an alarm rather than an
  instrument. The panel now declares 42 by 10 and hands everything past that to
  a neighbour, and the figures are pinned by a test so they cannot drift back.
- The README says what happens when you upgrade. A new widget does not appear
  in a config you already have — mirador never rewrites your config, which is
  what makes it safe to hand-edit, so a config written by an earlier version
  lays out the panels that existed when it was written and nothing since. The
  status bar has always named the widgets a layout does not place; the README
  now explains the notice, shows what it looks like, and gives the row to paste.
  The first person to add the pomodoro panel to an existing config went looking
  for a missing setting instead.
- The README's checksum command no longer prints a warning. `dist` writes its
  `.sha256` files with a trailing blank line, so `shasum -c` verified the
  archive and then added `WARNING: 1 line is improperly formatted` — true of
  the blank line, not of the hash, but a warning printed beside a checksum is
  the one place ambiguity is worst. Piping through `grep .` drops the blank
  line; both the macOS and Linux forms are tested to print `OK` and exit 0 on a
  good archive and `FAILED` and exit 1 on a tampered one. A Windows form is
  documented too, now that there is a Windows archive to verify.

## [0.2.0] - 2026-07-25

### Added

- Windows binaries. Releases now carry `x86_64-pc-windows-msvc` alongside the
  two macOS targets and Linux x86-64, plus shell and PowerShell installers and
  a source archive, all with checksums.

  `aarch64-pc-windows-msvc` is deliberately absent: `ring`, reached through
  `ureq`'s TLS, does not build for it. musl is absent for the same reason. The
  binary is built rather than exercised — nothing in mirador is Unix-specific
  and its dependencies are cross-platform, but that is a claim about compiling,
  not about behaving, and only the first has been checked.

### Changed

- `.github/workflows/release.yml` is generated by `cargo dist` and is no longer
  hand-edited; it is build output. The tag-versus-manifest check and the
  prerelease detection added a release ago are gone from it because `dist`
  provides both natively, and archive names lose their embedded version
  (`mirador-aarch64-apple-darwin.tar.gz`).

  `dist init` also wanted `[profile.dist] lto = "thin"`, which would have
  quietly undone the `lto = true` the release profile has always had. That
  override is removed, so the binaries do not get slower for changing how they
  are packaged.

## [0.1.0] - 2026-07-25

Initial release.

The Changed and Fixed sections record decisions reversed and defects found
*before* this first tag rather than after it. Nothing below was ever shipped
in an earlier version — they are kept because the reasoning is worth having.

### Added

- First run is no longer blank. The task list and notes seed a few examples
  when their file does not exist yet, and the examples are the documentation:
  they name the keys, carry a due date in each direction, a priority, a tag and
  a note, so the table has something to line up and an overdue row shows what
  overdue looks like. They are ordinary entries and `d` deletes them.

  Keyed on the file being *absent* rather than empty, so clearing them sticks —
  the opposite would make them impossible to get rid of. Their titles are held
  to what the task column can show at 120 columns, since an instruction
  truncated to `Press ? for every key, h…` is worse than no instruction.
- A startup hint naming widgets your layout does not place, on the right of the
  status bar, plus a section in the help overlay. A config written by an earlier
  version silently lacks every widget added since — an absent widget is a valid
  choice, so nothing errors and `--migrate-config` has nothing to fix, which
  left reading the release notes as the only way to discover a new panel. The
  status-bar notice is retired by the first keypress or click, because a
  dashboard you leave open all day must not nag; the help overlay keeps it, on
  the grounds that `?` is where you go when you wonder what else there is.
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

### Changed

- The weather panel declares a maximum width and hands the surplus to the
  clock. Once every forecast column is showing, more width only inflates the
  flexible sky column and pushes the numbers away from the labels they belong
  to. On a 193-column terminal the clock goes from 57 columns to 80 — enough to
  render block numerals where it had been falling back to plain text, which is
  the one thing that panel exists not to do.
- The weather forecast fills the height it is given: `[weather].forecast_hours`
  is a floor the panel is sized for, not a ceiling on what it may show, and the
  fetch retrieves a full day so a taller panel never waits for a refetch.
- The calendar stacks further rows of months into spare height, up to a year on
  screen. `[calendar].months` is now how many sit *across* — and so how wide the
  panel ever gets — rather than how many exist; it is a floor on the number
  shown, not a ceiling. The width cap stays, so the panel still hands surplus
  columns to its neighbours instead of spreading out.
- The stock watchlist declares a maximum width. Every column but the sparkline
  is fixed and the sparkline is capped, so past that the table only drifts
  apart; the columns go to the CPU and network graphs, which have no such limit.
- Panels may declare the size past which more space does nothing for them, and
  the layout hands their surplus to a neighbour that can use it. A clock cannot
  use a hundred columns and a calendar cannot use more than its months need;
  proportional layout gave them the space anyway and they sat in it while the
  task list next door ran out. The calendar and clock are bounded on both axes,
  the weather panel on height. When every panel in a row is bounded the maxima
  are ignored rather than leaving a gap — panels draw their own frames, so
  unallocated cells would show as a hole.
- `[weather].units` switches at runtime with `u`. The conversion is applied at
  render rather than re-requested, so the change is instant instead of putting a
  network round trip behind a keypress, and the forecast table converts with the
  readout above it.
- The note body sits below the list rather than beside it. Side by side splits a
  finite width between a list that wants room for titles and a body that wants
  room for prose, so neither got enough; stacking gives both the full width and
  spends height, which is the cheaper axis for both. `[notes].preview` takes
  `below` or `beside`, replacing `side_by_side_min_width`.
- CPU and network history buffers grow to fill the panel. `history` is a floor
  now, not a ceiling: the graphs pack two samples per cell and fill from the
  right, so a buffer of N samples could only ever cover N/2 cells and anything
  wider showed dead space on the left. The span readout is computed from the
  live sample count, so it stays honest as the buffer grows.

- The crates.io name is reserved: `mirador` `0.0.0` is published. Reservation is
  first-come with no reclamation and the name is an ordinary English word, so
  this was the one outstanding item with a deadline attached rather than a
  preference. `0.0.0` says reservation rather than release, which leaves `0.1.0`
  free for the first real one. The README says so plainly instead of letting the
  version number imply that a placeholder is a shipped product, and MSRV and
  platform badges now sit alongside the crates.io one.
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

- A config with no `[layout]` section no longer silently loses three panels.
  The Rust default described a five-widget dashboard while the shipped file
  described eight, so deleting a section you thought was redundant quietly took
  the calendar, notes and watchlist with it. They now describe the same
  dashboard, and a test fails if they drift apart again.
- The minimum supported Rust version is 1.95, not 1.85. The declared floor had
  stopped being true: `sysinfo` requires 1.95 and ratatui's tree requires 1.88.
  `Cargo.toml`, the CI toolchain, the README and CONTRIBUTING now agree.
- A rustdoc intra-doc link pointed at a `cfg(test)` item, which rustdoc cannot
  resolve; with `-D warnings` that failed the documentation build.
- Dependabot no longer proposes a Rust version that does not exist.
  `dtolnay/rust-toolchain` publishes a ref per Rust release rather than semver
  tags, and dependabot ordered them numerically: it read the MSRV job's
  `@1.95.0` pin and offered `@1.100.0`, which fails to install with a 404 from
  `static.rust-lang.org`. That pin tracks `rust-version` by hand and the other
  five uses are `@stable`, so version updates to that action are now ignored
  outright rather than closed by hand every month.
- Deleting a task no longer hands its id to the next one added. Ids came from
  `max(id) + 1`, so removing the highest-numbered task gave that number
  straight back, and anything still holding it — the selection, an open edit
  form, a pending delete confirmation — would silently act on a different task.
  The store now keeps a high-water mark that only climbs, rebuilt from the file
  on load, which is safe because nothing holds an id across a restart.
- A dropped column no longer slides every value after it under the wrong
  header. Row cells were indexed by *surviving* column position while callers
  pass them in declared order, so a narrow forecast showed the feels-like
  temperature under RAIN, and a narrow task list would have shown tags under
  DUE. Silent, and exactly the failure this module exists to prevent.
- Forecast columns appear as soon as they fit. Their thresholds were hand-tuned
  for a layout that no longer exists and were about fifteen columns too
  conservative — the table only completed at 62 when it fits at 47. They are
  now derived from one rule: a column appears once the grid can seat it and
  still leave the sky column its longest label. WIND now arrives at 47 rather
  than 62, RAIN at 33 rather than 38, FEELS at 39 rather than 52, and the
  weather panel's maximum width drops from 66 to 51, handing the difference to
  the clock.
- A failed weather refresh no longer blanks the panel. It used to replace the
  last good reading with an error, so on a dashboard left running for days —
  where a transient network blip is close to certain — one timed-out request
  cost the whole panel for a full refresh interval. The reading is kept and its
  age is shown instead: the border counter switches from the observation time
  to `2h old`, and the panel says so in its own body in amber. A reading is also
  called stale after twice the refresh interval even when nothing has failed,
  which catches a fetch thread that quietly stopped or a laptop resumed from
  sleep. Only a panel that has never loaded anything shows an error.
- The watchlist sparkline came back. Bounding the panel's width left the grid
  one column short of the threshold at which the sparkline column earns its
  place, so the column was dropped and the space sat empty. The threshold, the
  panel's maximum width and the width the sparkline is drawn at are now derived
  from one constant, and the panel asks the grid for the resolved column width
  instead of recomputing it — three copies of that arithmetic is what let them
  drift apart in the first place.
- Month names are centred over their own month. `centred` padded on the left
  but not the right, so a title line was short of the full block width and
  every month after the first slid left by the shortfall.
- The notes list and the reading pane have a rule between them. Without one the
  panel read as a single list whose last rows had gone strange: the two halves
  are the same kind of text in the same colours, so nothing else separated them.
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

[Unreleased]: https://github.com/jchultarsky/mirador/compare/v0.9.1...HEAD
[0.9.1]: https://github.com/jchultarsky/mirador/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/jchultarsky/mirador/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/jchultarsky/mirador/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/jchultarsky/mirador/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/jchultarsky/mirador/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/jchultarsky/mirador/compare/v0.5.2...v0.6.0
[0.5.2]: https://github.com/jchultarsky/mirador/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/jchultarsky/mirador/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/jchultarsky/mirador/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/jchultarsky/mirador/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/jchultarsky/mirador/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/jchultarsky/mirador/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jchultarsky/mirador/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jchultarsky/mirador/releases/tag/v0.1.0

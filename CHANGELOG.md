# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.2.0] - 2026-08-01

### Fixed

- **Panels no longer let the terminal cut a value in half.** A line built at its
  natural width and handed to the renderer is not truncated by mirador — it is
  truncated by the terminal, which keeps the cells that fit and drops the rest
  without saying so. The terminal cannot tell a value from a fragment, so it cuts
  wherever the edge falls, and a fragment of a value is not a smaller truth.

  Found by rendering every panel across a range of widths rather than by reading
  the code. Four panels were cutting values at ordinary terminal sizes:

  - the **network** readout showed a bare `↑` with no upload figure beside it at
    100 columns, and `6.4 KB` — a total — where `6.4 KB/s` was meant;
  - **weather** showed `humidity` with its percentage gone;
  - the **pomodoro** footer trailed off as `25m focus ·` at the shipped default
    of 120 columns;
  - the **calendar** cut a date down the middle, so `14` under THU became
    Thursday the 1st, with nothing on screen to say otherwise.

  Values are now dropped whole — a figure leaves with its unit, an arrow with the
  number it points at — and prose is ellipsised, so an abridged message says that
  it is. Where a reading would be dropped only because of the padding that keeps
  it from jiggling, the padding goes first. The calendar drops whole weekday
  columns instead of half a date.

- **The empty-state messages in the agenda and watch log lost a word** when the
  panel was narrow. Both were hand-broken into two lines that still read as a
  whole sentence — `Nothing has` above `since 00:30.` — so the omission was
  invisible. They wrap now.

- **A table could build a row wider than the panel it was resolved for.** Column
  widths are declared without reference to the total, so a pane narrower than
  their sum overflowed and the last value on the row lost its tail. Long-standing
  and only reachable at small sizes, but the same defect as the four above.

- The status bar and the arrange-mode legend were doing this arithmetic by hand,
  and one of them measured bytes rather than display cells.

## [1.1.3] - 2026-08-01

### Fixed

- **The agenda panel's "no calendar yet" message was cutting off the path.** That
  message exists to tell you where to put your `.ics`, and it stopped mid-path —
  `Looked in /var/folders/zj/blsvny` — which is not an instruction. It wraps now,
  as does the prose above it, which had been hand-wrapped for one panel width and
  clipped at every narrower one, and as does the failure reason when a calendar
  cannot be read.

  Same shape as the markets fix in 1.1.2: the terminal clips whatever does not
  fit, so anything built without asking how wide the panel is will eventually say
  something untrue or unreadable.

## [1.1.2] - 2026-07-31

### Fixed

- **The markets panel was showing wrong numbers in the default layout.** Its row
  was built at full width whatever the panel could show, and the terminal
  clipped whatever hung over the edge — so a change of `+52.07` was drawn as
  `+52.0`. Every column now has a width below which it is dropped instead, in
  order of expendability: the sparkline first, then the percentage, then the
  change, then the last price. A missing column reads as a narrow terminal; half
  a number reads as a different number.

  Even the symbol has a floor, because a ticker clipped to `BRK.` is as wrong as
  a price clipped to `+52.0`.

- **The markets panel gains two columns in the shipped layout**, taken from the
  cpu graph, which scales to whatever it is given. Without them the fix above
  would have cost the default dashboard its change column — and the change is
  the answer to "what is the portfolio doing", which is one of the four
  questions this dashboard exists to answer. Existing configs are untouched; a
  config is seeded once and never rewritten.

## [1.1.1] - 2026-07-31

Working notes and test coverage. **Nothing you can see changes** — the binary
behaves identically, and the README and shipped config are untouched. Published
so the source on crates.io matches the repository.

### Changed

- The working notes now record what shaped the two reset flags: why they are
  separate commands rather than degrees of one, why resetting the config has to
  clear the remembered preferences with it, and why a factory reset decides what
  it may touch by which program wrote the file rather than by where it sits.
- A test now checks those notes for the kinds of staleness a machine can see — a
  test cited by name that no longer exists, a released version disagreeing with
  the manifest, a cited path that has moved. It found one on its first run. Most
  of that file is prose about the world and stays unchecked, which the test says
  out loud rather than implying otherwise.

## [1.1.0] - 2026-07-31

### Added

- **`mirador --factory-reset` puts everything back to how it arrived.** The
  reset `--reset-config` could never honestly be: it resets configuration, while
  your watchlist, tasks and notes outlive it. This sets aside every file mirador
  has written — config, remembered preferences, tasks, notes, watchlist and
  world clocks — and the next launch seeds them all again, default stock tickers
  included. A reset install is now indistinguishable from a new one.

  **Nothing is deleted.** Every file is renamed to a `.bak` beside itself, so it
  is something you can walk back from with `mv`, and an existing backup is never
  overwritten. That is also why a plain `y` is enough to confirm it: the prompt
  lists every affected file by full path, and the worst outcome is renaming
  things back rather than lost work.

  Your calendar is not touched. mirador only ever *reads* an `.ics`, so that
  file is yours even when it sits in mirador's own directory — and neither are
  files at paths you chose yourself with `[todo].file` and its siblings.

## [1.0.4] - 2026-07-31

### Fixed

- **`--reset-config` now resets what you can see.** It restored the config
  correctly all along, but left `state.toml` — the preferences you change from
  the keyboard — in place. Those *outrank* the config, so they were applied
  straight back over the file just restored, and the dashboard came back looking
  exactly as before. Anyone who had only ever changed their theme saw the
  command appear to do nothing. The remembered preferences are now put aside
  too, into `state.toml.bak`, and a second reset does not overwrite the first
  copy.

  Both the prompt and the result say what is kept, because the boundary was
  invisible from the flag's name: **your tasks, notes and watchlist are left
  alone.** They are your content rather than configuration, so a command called
  `--reset-config` does not delete them — which also means the default stock
  tickers are not restored. `[stocks].symbols` seeds `watchlist.toml` only when
  that file is absent, and that is what lets the panel edit the list at all.

## [1.0.3] - 2026-07-31

Documentation only. No behaviour changes at all; this exists so the corrected
text reaches the README on crates.io and the config mirador prints.

### Changed

- **The resize key is spelled the same everywhere.** The status bar, arrange
  mode and `--help` all say `Ctrl+arrows`; the README said `Ctrl+arrow` in four
  places, including both key tables, so the page you read and the bar you look
  at disagreed about one key. The comment at the head of the shipped config said
  it too, and that text is compiled into the binary, so `mirador --print-config`
  carried it. The singular survives where it means one keypress — that is
  grammar, not inconsistency.

## [1.0.2] - 2026-07-31

Housekeeping. Nothing you can see changes; both entries are things that were
wrong underneath.

### Fixed

- **A very long watchlist collapsed the stocks panel instead of filling it.**
  The panel reports the height it can use, and that sum was not saturating while
  already reaching for `u16::MAX` — so a watchlist of around 65,000 symbols
  wrapped it to three rows. Absurd in practice; the watchlist is a file you
  edit, so nothing bounded it. The cap itself was checked at the same time and
  is exactly right: the panel is complete at header, every symbol and the status
  line, and any extra height would be a blank gap, so the space goes to a
  neighbouring row instead.

### Changed

- **The test suite no longer reaches the network.** It was documented as never
  doing so, and that had quietly stopped being true: constructing the stocks
  panel spawns a fetch thread, so *building* one called Yahoo Finance — and
  since the default layout places that panel, any test building a dashboard did
  too. Quote sources are injectable now, and the one function that opens a
  socket refuses outright under `cfg(test)`. No effect on the shipped binary,
  which fetches exactly as before.

## [1.0.1] - 2026-07-31

### Fixed

- **The resize keys are advertised where you would look for them.** `Ctrl+arrow`
  has resized the focused
  panel since long before 1.0, but it was declared as a secondary binding, so it
  appeared only in the `?` overlay — someone looking for it on the status bar
  concluded the feature did not exist. It now sits beside the other primary
  hints, spelled `Ctrl+arrows resize`, the way arrange mode and `--help` already
  spelled it. A panel that cannot use the extra space still declines it, so
  growing a row may move the space to a neighbour rather than to the panel you
  are pointing at; that is deliberate.
- **The status bar no longer cuts a hint in half.** It drew every hint and let
  the terminal clip the last one, which on a narrow window left fragments like
  `Ctrl+←` — a rendering fault to look at, where a missing hint just reads as a
  narrow terminal. It now drops whole hints, which is what arrange mode's legend
  has always done. Reachable before this release for any terminal narrow enough
  to cut `t theme`.

## [1.0.0] - 2026-07-30

**1.0 is a promise, not a feature.** Nothing is added here that was not in
0.19.0. What changes is what the version number commits to:

- **Your config keeps working.** No option that has ever shipped has been
  removed. Re-verified before tagging against all thirty-one release tags —
  1860 key comparisons, nothing lost. The four keys predating `0.1.0` are still
  handled by `mirador --migrate-config`.
- **Your data files keep working.** `todos.toml`, your notes, `watchlist.toml`,
  `zones.toml` and `state.toml` have not changed shape since `0.1.0`, and all
  ignore keys they do not recognise — so a file written by a newer mirador still
  opens in an older one.
- **No known crashes or hangs.** Every module has been read adversarially, the
  untrusted-input boundary is bounded on every side, and the dashboard has been
  soaked across real midnights on macOS, Linux and Windows.

Breaking changes from here get a major version. Options may be added; they will
not be renamed out from under you.

## [0.19.0] - 2026-07-30

### Added

- **A story's link can now be copied or opened, not just looked at.** `y` asks
  the terminal to put it on the clipboard (OSC 52) — no configuration and no
  dependency, though some terminals refuse it and tmux needs `set-clipboard on`;
  mirador cannot tell either way, so it says it *sent* the link rather than
  claiming it copied one. `↵` opens it with `[news].open_command`, which is empty
  by default — mirador launches nothing you did not name, runs it directly rather
  than through a shell, and passes the link as its own argument so nothing in a
  URL can be read as shell syntax.
  [#137](https://github.com/jchultarsky/mirador/issues/137).

## [0.18.2] - 2026-07-30

### Fixed

- **The watch log erased its own "since you were here" line the moment you came
  back.** It marked the log seen on gaining focus as well as losing it, so
  returning to the dashboard set "last looked" to *now* — everything that
  arrived while you were away landed on the old side of the line, and the line
  disappeared in the instant you came back to read it. A terminal that reported
  focus *correctly* made the feature less visible than one that did not. Only
  losing focus marks it seen now.
  [#132](https://github.com/jchultarsky/mirador/issues/132).

- **The clock's reorder keys were invisible.** `Shift+↑`/`↓` shipped in 0.17.0 as
  an `extra` binding, which put it in the help overlay and nowhere else — so the
  panel border advertised `e edit` and said nothing about reordering, the thing
  [#109](https://github.com/jchultarsky/mirador/issues/109) was actually asked
  for. It is now a primary and sits above `d remove` on the border. `a add zone`
  is shortened to `a add` to make room; `d` still deletes and still appears on a
  wider panel, and it is the key every other list panel already teaches.

## [0.18.1] - 2026-07-30

### Changed

- **A story's link no longer hides in plain sight.** `o` drew it in the same
  verdigris every masthead wears, so it was camouflaged by repetition and
  appeared without drawing the eye. It is now brass — the dashboard's attention
  colour — and prefixed `↳`, with wrapped lines aligned under it.
  [#117](https://github.com/jchultarsky/mirador/issues/117).

### Fixed

- **`o` did nothing until you moved the cursor.** The selection starts empty, so
  on a freshly focused news panel the key was silently ignored while the border
  advertised `o show link` regardless. It now shows the top story's link and
  marks the story it came from.

## [0.18.0] - 2026-07-30

### Added

- **Arrange mode can move a row, not just a panel.** `Shift+↑`/`↓` (or `J`/`K`)
  moves the whole row the focused panel sits in. Before this, a new row was only
  ever created by pushing a panel off the top or bottom edge, so a panel alone in
  a *middle* row could not travel at all — `Down` merged it into its neighbour
  and the row count fell. Going from `[clocks] [watchlog] [notes] [cpu]` to
  `[clocks] [notes] [watchlog] [cpu]` was not expressible with any sequence of
  keys. Moving a panel between rows still merges, exactly as before.
  [#100](https://github.com/jchultarsky/mirador/issues/100).

## [0.17.1] - 2026-07-30

### Fixed

- **The news panel scrolled, which its own rule said it must not.** Every story
  became a list item, so the selection could walk past the bottom of the panel
  and ratatui's `List` scrolled to follow it — with the shipped feeds that made
  nine of twelve stories reachable only by scrolling. "However many stories fit,
  and no more" was documentation rather than behaviour, and nothing tested it.
  The panel now builds only the stories whose whole block fits, so the cursor
  cannot leave the viewport and there is nothing to scroll. A taller panel still
  shows more. [#118](https://github.com/jchultarsky/mirador/issues/118).

## [0.17.0] - 2026-07-30

### Added

- **The clock's zone list can be reordered, edited and located.** `Shift+↑`/`↓`
  (or `J`/`K`) moves the selected clock through the table, `e` opens it in the
  same `Label = Zone` dialog `a` uses, pre-filled, and `o` shows where
  `zones.toml` lives. Previously the only way to change an order or fix a label
  was to delete entries and re-add them in the order you wanted, which is what
  the reporter did. The first entry is still the big clock and still cannot be
  displaced — reordering the table is what was asked for; choosing the primary
  is a different decision and would want its own key.
  [#109](https://github.com/jchultarsky/mirador/issues/109).
- **`--reset-config`, a way out of a config that has gone past fixing.** Writes
  the shipped defaults and copies the old file to `config.toml.bak` first. The
  name sounds harmless and the effect is not, so it says what it is about to do
  and waits for a `y`; piped somewhere with no terminal to ask on, it refuses
  rather than assuming, and `--yes` is there for scripts that mean it. An
  existing backup is never overwritten — resetting twice is exactly what a stuck
  reader does, and with a fixed name the second run would have replaced the real
  config with the defaults written by the first.
  [#111](https://github.com/jchultarsky/mirador/issues/111).

### Fixed

- **The agenda kept saying `reloading…` after the reload had finished.** The
  status was cleared only by the next keypress, so a panel nobody touched went
  on claiming to be mid-operation — measured at 83 seconds with the reloaded
  events already on screen. A dashboard is read without being touched, so
  "cleared on the next keypress" was, for anyone glancing at it, never.
  [#120](https://github.com/jchultarsky/mirador/issues/120).
- **The watch log told you to set a calendar you had already set.** It read
  `[agenda].file` once at construction, so a calendar added later with `f` was
  never noticed and the panel went on advertising `f` until a restart. It now
  says where calendar entries come from and asserts nothing about whether you
  have one. [#119](https://github.com/jchultarsky/mirador/issues/119).

## [0.16.3] - 2026-07-30

### Fixed

- **The news panel did not show which story the cursor was on.** It kept a
  selection that `j`/`k` moved and that `o` read the link from, and drew no
  highlight at all — so the link at the foot of the panel belonged to a story
  you had no way to identify. The task, notes and watchlist panels have always
  marked their selection; this one never did.
  [#114](https://github.com/jchultarsky/mirador/issues/114).

## [0.16.2] - 2026-07-30

The first round of bug reports from people who are not the author, and one
defect found while confirming one of them.

### Fixed

- **Hiding the seconds left them showing in the zone table.** `s` is bound to
  "seconds" and the clock is one panel, but the secondary zone list formatted
  from `[clocks].time_format` and never consulted the setting — so `s` gave you
  a clock with no seconds directly above a table that still had them. Your own
  format survives it: the seconds specifier is removed from *your* format rather
  than swapped for a fixed one, so `%I:%M:%S %p` stays a 12-hour clock. Reported
  by email; [#106](https://github.com/jchultarsky/mirador/issues/106).

- **The `+1d` / `-1d` day marker was silently truncated out of the zone table.**
  The column held the time *and* the marker in nine cells, and `02:43:48 +1d` is
  twelve — so the marker was cut every time a zone was on a different date,
  which is every time it matters. It is the half of that row that carries the
  warning, and the reason it exists is that a day boundary is the thing people
  get wrong. Found while confirming the above, not reported;
  [#107](https://github.com/jchultarsky/mirador/issues/107).

- **Pressing `o` on a news story cut the URL off.** It was truncated to the panel
  width, so the link could not be read or copied — and the terminal linkified the
  visible text, which now ended in an ellipsis, so clicking it went to a URL that
  does not exist. The footer now takes the rows the whole link needs. Reported by
  email; [#108](https://github.com/jchultarsky/mirador/issues/108).

## [0.16.1] - 2026-07-29

A hotfix for one reported bug, and a second one of the same shape found beside
it.

### Fixed

- **Hiding the seconds on the clock made the time render in small text.**
  Pressing `s` to drop the seconds should make the numerals larger if anything;
  instead the clock could fall back to plain text entirely.

  The scale search took only a width, and the callers filtered its answer by
  height afterwards — which rejects rather than stepping down a size. Because a
  shorter string fits a *bigger* scale, and each scale is five rows taller than
  the last, `HH:MM` could earn a scale that was wide enough but too tall, and
  lose its block numerals altogether. Reproduced at ordinary sizes: any terminal
  around 74-110 columns by 15-17 rows with the clock panel given the width.
  **Present since 0.1.0.** Reported by @abusch in
  [#103](https://github.com/jchultarsky/mirador/issues/103).

- **The pomodoro timer could lose its numerals partway through a session**, for
  the same reason and in the same helper. A focus period over 99 minutes renders
  `180:00` and counts down to `99:59`, and the shorter string could earn a scale
  too tall to draw. Found while fixing the clock rather than reported.

## [0.16.0] - 2026-07-28

Four adversarial review passes, completing the first phase of the work towards
1.0. Two of them found crashes, one found a way for a third-party news feed to
grind the dashboard down, and one found a bug in code three days old.

### Removed

- **The unused-widget notice.** mirador used to name the widgets your layout
  does not place — once in the status bar at startup, and permanently in the
  help overlay. The status bar line retired on your first keypress; the overlay
  section did not, so if you had deliberately switched four panels off you were
  told about them every time you pressed `?`.

  A dashboard cannot tell "has not discovered this yet" from "decided against
  it", so a hint aimed at the first reminds the second for ever. `w` is still on
  the status bar and in the help, so the way to switch a panel on is unchanged;
  what is gone is being told that you should.

### Changed

- **The default watchlist leads with the major US indexes** — S&P 500, Dow,
  Nasdaq Composite, Russell 2000 and Nasdaq-100 — keeping `AAPL` and `MSFT` so
  the panel shows both kinds on a first run. This seeds the watchlist file on a
  *first run only*, so an existing installation is untouched.

- **News headlines are clipped to 400 characters when read**, links to 1,000 and
  feed names to 80. See below for why; no real headline comes close.

### Fixed

- **A one-column terminal could bring the dashboard down.** The prompt dialog
  computed its cursor position as `popup.x + popup.width - 2`, which underflowed
  once the popup was clamped to a screen narrower than itself. Reachable by
  resizing your terminal while a prompt is open.

- **A news feed could decide how much work mirador does.** Nothing bounded the
  text read out of a feed, and everything downstream of a story runs on every
  frame — the headline is wrapped, the feed name uppercased. A 2 MB headline
  wrapped to 40,000 lines and cost **72ms a frame**, and the HTTP body limit
  allows five times that. A feed is the one input to this program that somebody
  else writes.

- **Place names and paths in non-Latin scripts drew over their own edges.** The
  text field measured its scroll window in *characters* rather than display
  cells, so a six-column field holding `北京市中心` decided five characters fit
  and drew ten cells, with the caret landing in the middle of a glyph.

- **A long line in a note body took the cursor off the screen.** The editor
  deliberately does not wrap — a soft wrap that moves as you type makes the
  cursor impossible to follow — and that had quietly come to mean you could keep
  typing past the right-hand edge and see none of it. It scrolls sideways now.

- **Arrange mode could rescale rows you had not touched.** Pushing a panel off
  the edge of a thin row grew the total of the layout weights, so a dashboard
  written as two equal rows became three unequal ones. Layouts written without
  explicit heights were the ones affected.

- **A list that got shorter left its cursor stranded.** Pressing Up walked the
  selection down from wherever it had been, one row at a time, with nothing
  highlighted the whole way. Down had always pulled it back into range; both do
  now.

- **`Alt` and a letter typed the letter into a note body**, where the task title
  field had always ignored it.

- **Moving the text cursor in the timezone picker threw away your place in the
  list.** Left and Right were handled as though you had typed.

- **A panel graph handed an area larger than the screen panicked** rather than
  drawing what fits, and a sample buffer given a capacity of zero spun for ever.
  Neither was reachable from any current caller; both are closed.

- **Rearranging a layout written with `[[layout.rows]]` sections** said only
  "no `[layout]` rows found", about a file that visibly has rows. It now names
  the form it can rewrite. That form is still the only one mirador edits.

## [0.15.0] - 2026-07-27

### Added

- **Six more themes, and a `t` key to try them on.** Nord, Gruvbox, Dracula,
  Catppuccin Mocha, Tokyo Night and Solarized Dark now ship inside the binary
  alongside mirador's own four, so the dashboard can match whatever your editor
  and terminal are already wearing. The values come from each palette's own
  specification and the theme file cites its source: these are ports, not
  interpretations.

  Press `t` to browse them. **The list previews as you move through it**, on
  your real dashboard rather than on a swatch, because a theme you cannot see is
  a theme you cannot choose. `Enter` keeps what is on screen; `Esc` puts back
  what you had. Themes of your own are listed alongside the shipped ones and
  marked as yours.

  Your choice is remembered the same way your weather units and sort order are.
  If you set `theme` in your config and it seems to be ignored, you picked
  something else with `t` at some point — pick it again, or delete the state
  file.

  All six keep `text = "reset"`, so body text still follows the foreground you
  have already tuned your terminal to.

### Fixed

- **A theme name is a name, not a path.** `theme = "../../elsewhere"` resolved,
  reading and parsing a file outside your themes directory. Nothing escalated —
  the config and anything it could reach are yours — but a name whose meaning
  depends on where your config sits is not a name.

- **Two mirador windows no longer take each other's saves away.** Every writer
  of a file used the same `.tmp` name, so whoever renamed second found it gone.
  Measured with eight concurrent writers: **2,100 of 2,400 saves failed** — and
  a failed save is reported, so the second window filled with "could not be
  saved" for no reason. Temporary names are now unique per write.

- **A file you restricted stays restricted.** A save replaces the file, and the
  replacement was created per the umask — so `chmod 600` on your tasks was
  silently widened to world-readable the next time you added one.

- **An untouched dashboard no longer writes to its state file.** `[agenda].file`
  ships commented out, so the baseline was empty while the panel reported a
  resolved path; one keystroke anywhere pinned that path into `state.toml`,
  after which setting `[agenda].file` in the config did nothing, because the
  state file outranks it. That is the failure invariant 17 exists to prevent,
  reached by a new route.

- **The agenda cloned its whole event list three more times per frame.** Once in
  `render`, once in `counter` — which the frame renderer calls on *every* frame
  with nothing guarding it — and twice more in key handlers that cloned the list
  only to read its length. Measured against a calendar of three hundred daily
  meetings: **210,000 event clones in thirty idle seconds**, now zero. A
  recurring rule expands, so the list is far longer than the file looks.

- **A `.ics` larger than 10MB is refused rather than read.** The network side
  was bounded and the local side was not, and reading a calendar costs more than
  its size — unfolding makes a `Vec<String>` of it and recurrence expands it
  again.

- **Editing a config on Windows no longer rewrites every line in it.** Both
  places that rewrite a file you wrote by hand — the layout editor and the
  config migrator — reassembled it from `str::lines()`, which strips the `\r`
  of a CRLF ending. Joining with `\n` then converted the whole file to LF: you
  moved one panel and git reported every line as changed. The ending the file
  already uses is preserved now, in both directions.

## [0.14.1] - 2026-07-27

### Fixed

- **A headline containing a wide character could freeze the dashboard.** Text
  wrapping split an over-long word by taking as many characters as fit — and for
  a CJK glyph or an emoji in a column one cell wide, none fit, so nothing was
  consumed and the loop ran for ever. Any news headline with such a character in
  a narrow panel would hang mirador until it was killed. **Present in 0.13.0 and
  0.14.0.**

- **The agenda cloned its whole event list on every frame.** The on-fire signal
  asks each panel whether anything is urgent, and the agenda answered by copying
  every event first. Measured: 456 event clones in thirty idle seconds against a
  twelve-event calendar, scaling with the size of the calendar. Now zero.

- **The news panel cloned every story on every frame**, for stories that change
  once an hour. Copied when the fetch lands instead.

- **The README said mirador has two network panels.** It has three.

## [0.14.0] - 2026-07-27

### Added

- **The status bar says when something needs you.** One line, naming the single
  most pressing thing — an event about to start, a save that is failing, a
  layout change that did not persist — and nothing at all when nothing is
  pressing.

  It names one thing and never a count, and it clears itself when the cause
  does. **There is no all-clear**: an indicator saying everything is fine is a
  light you have to read to learn nothing.

  Deliberately strict about what qualifies: *will this get worse if nobody acts
  in the next few minutes*. An overdue task does not — it is notable, already
  red in its own panel, and a signal lit for it would be lit permanently, which
  is how a warning becomes furniture.

### Fixed

- **A layout that could not be saved now says so.** It was reported inside the
  `w` picker and nowhere else, so a rearrangement that failed to persist was
  silent once the picker closed — and gone at the next launch. It was the one
  genuine silent failure left in the program.

## [0.13.0] - 2026-07-27

### Added

- **A news panel.** Headlines from RSS feeds you choose, refreshed hourly.

  **A window, not a feed:** however many stories fit and no more, with no
  scrolling, no count, no unread state and nothing to dismiss. News is the
  doomscroll surface this dashboard has been avoiding, and that commitment is
  what makes it something you glance at rather than something you work through.

  Stories are interleaved across feeds so the top of the panel holds the newest
  from *each* — date order alone hands the whole window to whichever outlet
  publishes most often.

  `o` shows a story's link so you can copy it; no browser is launched. The
  shipped feeds are science, space and technology only, because choosing
  outlets for general news is an editorial act this project should not make for
  you. Headlines only — feed summaries are article prose belonging to whoever
  wrote them.

- **The watch log records the day turning.** It is the one source that always
  fires, so the panel has something to say before you point it at a calendar,
  and it doubles as the divider marking where one day's entries end. Recorded by
  the shell rather than a panel: the todo panel notices a rollover too, but a
  day-divider that vanishes when you switch off the task list would be odd.

- **An empty watch log says what it is watching.** It had no refresh key —
  nothing there is polled, the panels report to it — and "Nothing has happened"
  on its own gave a reader no way to tell working from dead. It now names the
  two things it watches, and says plainly when no calendar is configured that
  only one of them can happen.

- **The default layout is four rows**, with news and the watch log sharing a
  reading row. Both want width for prose rather than columns of numbers, and
  squeezing them in beside the lists left neither readable.

## [0.12.0] - 2026-07-27

### Added

- **The version is shown in the `?` overlay**, right-aligned in its border the
  way every panel shows a counter. `?` is where you go to find out what the
  thing does, so it is where you look for what version it is.

- The command-line `--help` now leads with the version, and lists `w`, `m` and
  `Ctrl+arrows`, which it had never been told about.

### Changed

- **Adding a clock offers a list of cities instead of asking for an identifier.**
  Type to narrow it, `↑↓` to choose. Matching runs over the city *and* the
  identifier, anywhere in either, so `seattle` finds `America/Los_Angeles` —
  which is the whole point: the identifier names *a* city in the zone and it is
  very often not the one you have in mind. Bengaluru is `Asia/Kolkata`, Boston
  is `America/New_York`.

  The city you picked becomes the clock's label, and the identifier is shown
  beside it so what lands in `zones.toml` is never a surprise. A zone the list
  does not carry is still taken as typed.

### Fixed

- **A panel's prompt is no longer drawn inside that panel.** The agenda's file
  prompt was as narrow as the agenda, which for a long path meant reading a
  scrolled fragment through about forty columns. Prompts are drawn by the shell
  over the whole terminal now, after every panel — which is also what stopped
  the new city list coming out interleaved with the task list.

- **The city list scrolls.** It draws ten rows, and moving the selection past
  the bottom kept moving a cursor nobody could see — the highlight vanished and
  the row `Enter` would take was anybody's guess. The window follows the
  selection now, `PageUp`/`PageDown`/`Home`/`End` work, and the help line says
  how much of the list is on screen so ten of a hundred and forty-three does not
  read as all of it.

## [0.11.0] - 2026-07-27

### Added

- **The watch log** — a placeable panel recording what happened while you were
  not looking. It fills the third instrument the design thesis named
  (*"chronometer, weather glass, watch log"*) and that nothing had ever
  occupied.

  Almost nothing qualifies for it, which is the point. The clock, the readings,
  the prices and the graphs change continuously and none of it is news; your
  notes and tasks change because you changed them. An entry is something that
  happened *to* you rather than because of you, and that you would want to know
  even if you never looked at the panel it came from. Out of the box that is two
  things: an event appearing in your `.ics` that you did not add, and a task
  crossing into overdue because the day turned.

  **No counter, no unread state, no effect on any other panel.** Unread message
  counts were considered for this dashboard and rejected as a doomscroll hook;
  read closely, that objection is about the *badge* — a number that accumulates
  and demands you zero it. This is a record you consult, not an inbox that
  consults you, and nothing in it can be dismissed, because an entry you can
  dismiss is an entry you are expected to dismiss.

  A rule line marks where you were last seen. mirador now asks your terminal to
  report window focus, which is the only honest signal for "the reader is here";
  it falls back to your last keypress, and draws no line at all when neither has
  fired, because a line in the wrong place makes a claim that a missing one does
  not.

  The log lives in memory and says when it started watching. One written to disk
  would return after a restart with a gap it could not mark.

## [0.10.0] - 2026-07-27

### Added

- **Named themes.** `theme = "high-contrast"` in place of the `[theme]` table.
  Four ship inside the binary — `default`, `default-light`, `high-contrast` and
  `ansi` — and anything in `themes/<name>.toml` beside your config is found
  first, so a bundled theme can be replaced without renaming it.

  A theme file may `inherits` another, so a variant is the lines that differ
  rather than a copy of all eighteen keys, and may define a `[palette]` of named
  colours. Redefining a palette entry in a child recolours the keys its *parent*
  set with it.

  The same key does both jobs and TOML decides which: a quoted string is a name,
  a table is colours written out. Your existing `[theme]` table keeps working
  and keeps its error messages — the misspelled-key report and the
  `--migrate-config` hint for the pre-0.1.0 `rx`/`tx` keys both survive, which
  an untagged enum would have flattened into "data did not match any variant".

  Not built, deliberately: dotted-scope fallback (sized for Helix's hundreds of
  syntax scopes, where mirador has thirteen flat semantic keys) and per-key
  style objects with fg/bg/modifiers (every theme read takes a flat colour, and
  emphasis belongs to the widget that knows what it is emphasising).

- **A theme file with its colour keys after `[palette]` is refused by name.**
  TOML puts every key following a table header *inside* that table, so those
  keys set nothing — without failing, without a typo, and with the theme quietly
  coming out as the defaults. mirador's own `default.toml` was written that way
  during development and the test comparing it against the built-in default
  passed, because a file that sets nothing resolves to exactly the default.

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

[Unreleased]: https://github.com/jchultarsky/mirador/compare/v1.1.3...HEAD
[1.1.3]: https://github.com/jchultarsky/mirador/compare/v1.1.2...v1.1.3
[1.1.2]: https://github.com/jchultarsky/mirador/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/jchultarsky/mirador/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/jchultarsky/mirador/compare/v1.0.4...v1.1.0
[1.0.4]: https://github.com/jchultarsky/mirador/compare/v1.0.3...v1.0.4
[1.0.3]: https://github.com/jchultarsky/mirador/compare/v1.0.2...v1.0.3
[1.0.2]: https://github.com/jchultarsky/mirador/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/jchultarsky/mirador/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/jchultarsky/mirador/compare/v0.19.0...v1.0.0
[0.19.0]: https://github.com/jchultarsky/mirador/compare/v0.18.2...v0.19.0
[0.18.2]: https://github.com/jchultarsky/mirador/compare/v0.18.1...v0.18.2
[0.18.1]: https://github.com/jchultarsky/mirador/compare/v0.18.0...v0.18.1
[0.18.0]: https://github.com/jchultarsky/mirador/compare/v0.17.1...v0.18.0
[0.17.1]: https://github.com/jchultarsky/mirador/compare/v0.17.0...v0.17.1
[0.17.0]: https://github.com/jchultarsky/mirador/compare/v0.16.3...v0.17.0
[0.16.3]: https://github.com/jchultarsky/mirador/compare/v0.16.2...v0.16.3
[0.16.2]: https://github.com/jchultarsky/mirador/compare/v0.16.1...v0.16.2
[0.16.1]: https://github.com/jchultarsky/mirador/compare/v0.16.0...v0.16.1
[0.16.0]: https://github.com/jchultarsky/mirador/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/jchultarsky/mirador/compare/v0.14.1...v0.15.0
[0.14.1]: https://github.com/jchultarsky/mirador/compare/v0.14.0...v0.14.1
[0.14.0]: https://github.com/jchultarsky/mirador/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/jchultarsky/mirador/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/jchultarsky/mirador/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/jchultarsky/mirador/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/jchultarsky/mirador/compare/v0.9.1...v0.10.0
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

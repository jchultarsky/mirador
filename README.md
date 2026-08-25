# mirador

[![CI](https://github.com/jchultarsky/mirador/actions/workflows/ci.yml/badge.svg)](https://github.com/jchultarsky/mirador/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/mirador.svg)](https://crates.io/crates/mirador)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.95+](https://img.shields.io/badge/rust-1.95%2B-dea584.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)](#install)

An opinionated personal dashboard for your terminal: world clocks, a calendar,
weather, a real task list, notes, a market watchlist, and live CPU and network
charts, laid out how you want them.

It is built for one job — to sit in a terminal tab you leave open all day and
glance at. That single constraint drives every decision below: nothing blinks,
nothing shimmers, and nothing is designed to pull you back to it.

A *mirador* is a lookout — the tower you climb to see everything at once.

![All thirteen mirador panels on a wide terminal, in use: a block-numeral clock, two months of calendar, weather with an hourly forecast, the task list, an agenda of upcoming events, notes, a news panel of headlines, a watch log, a calculator, a pomodoro timer, and a market watchlist beside live CPU and network graphs. Focus moves between panels, a task is typed in and added to the list, the panel picker switches a panel off and the grid reflows around it, arrange mode moves the task panel along its row and up into the row above until it takes a row of its own, the help overlay opens and scrolls, three sums are typed into the calculator and feed up its tape, and the pomodoro timer starts counting down](https://raw.githubusercontent.com/jchultarsky/mirador/main/docs/demo.gif)

*All thirteen panels on a first run, at 200x50 — wide enough that nothing has to fall
back. The lit frame is the focused panel, with its own keys in its bottom
border; every other panel is dimmed, so exactly one thing is at full brightness.*

*A minute of it in use. Focus moves with `Tab`; a task is typed in and
lands in the list, which goes from four open to five. `w` opens the panel picker,
switches one off, and the grid closes over the gap. `m` starts arranging: the
task panel moves along its row, then up into the row above, then past the top
edge into a row of its own — the real panels move as the keys are pressed, and
`Esc` puts it all back. `?` lists every binding for the focused panel and
scrolls when there are more than fit. Then three sums go into the calculator:
each answer appears in the right-hand column as it is typed, and Enter feeds it
up the tape, where the results line up on their decimal points.*

*The tasks and the note are the examples mirador seeds on a first run, and the
weather, the prices and the graphs are whatever was true while it recorded —
`docs/record-demo.sh` drives a real build under tmux. The one staged thing is
the calendar, because mirador deliberately never invents one: without a sample
the agenda panel would spend the recording explaining how to point it at a file.*

## Why

Most terminal dashboards are built to be *watched* — a system monitor you open
when something is wrong, dense and busy and refreshing as fast as it can.
mirador is built to be **glanced at**: one tab, all day, in the corner of your
eye. Everything follows from that. Nothing blinks or shimmers. The unfocused
panels dim so exactly one thing is at full brightness. A notice retires on your
first keypress, because a dashboard that nags is a dashboard you close.

**Your data stays in files you own.** Tasks, notes and the watchlist are plain
TOML you can hand-edit, keep in git, or delete. No database, no sync service,
no account, no API key, no telemetry, and nothing phones home unless you ask it
to — including the updater, which runs only when you run it, and an update check
that is off until you turn it on.

**It is a real task list, not a checklist.** Due dates, priorities, notes, tags,
filtering and full editing without leaving the dashboard. That is the panel most
dashboards treat as a checkbox, and it is the one you will use most.

The three network panels use free key-less endpoints:
[Open-Meteo](https://open-meteo.com) for weather, Yahoo's public chart endpoint
for prices — see [Market data](#market-data) for what that one means for you —
and, for news, whichever RSS feeds you configure. The shipped feeds are NASA,
Phys.org and Ars Technica; change or empty them in `[news.feeds]`.

Those three fetch the data their panels exist to show, which is not the same as
phoning home: nothing is sent about you, and switching a panel off with `w`
stops its requests entirely.

## How this was built

**mirador is written with heavy AI assistance.** The implementation, the tests
and most of this documentation were written by Claude (Anthropic), working from
my direction and review. The product decisions — what to build, what to leave
out, what the thing is for — are mine.

This is stated plainly because anyone evaluating the code deserves to know how
it was made, and because some communities reasonably want to know before they
adopt or recommend something. It is not a claim about quality in either
direction: judge it the way you would judge any other code.

What that means in practice, if you want to weigh it:

- Every change lands through CI that runs the test suite on macOS, Linux and
  Windows, plus `clippy` with `-D warnings`, `rustdoc` with `-D warnings`,
  a minimum-supported-Rust-version check, `cargo publish --dry-run` and
  `cargo-deny` for advisories, licences and sources.
- Behavioural claims in the commit history are measured rather than asserted,
  and several fixes were verified by reverting them to confirm the new test
  actually failed. Two tests in this repository were found to be pinning the
  bug they existed to catch.
- `CLAUDE.md` records the design decisions and the reasoning behind them,
  including the ones that were reversed and why.

## Install

From crates.io, if you have Rust:

```sh
cargo install mirador
```

Without Rust, on macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jchultarsky/mirador/releases/latest/download/mirador-installer.sh | sh
```

and on Windows:

```powershell
irm https://github.com/jchultarsky/mirador/releases/latest/download/mirador-installer.ps1 | iex
```

Both fetch the right archive for your platform and unpack the binary. If you
would rather read the script before running it, it is an ordinary file at that
URL, and the manual route is below.

The shell installer checks the archive against its published sha256; the
PowerShell one does no verification, and neither verifies the updater binary.
That is [dist](https://axodotdev.github.io/cargo-dist/)'s behaviour rather than
ours, and [SECURITY.md](SECURITY.md#verifying-a-download) says what it does and
does not buy you. Every artifact carries a GitHub provenance attestation, which
is the stronger check and the one to run if you care:

```sh
gh attestation verify --repo jchultarsky/mirador mirador-aarch64-apple-darwin.tar.gz
```

Whether you installed with Cargo or one of those installers, a later upgrade is:

```sh
mirador --update
```

The flag hands an installer-managed copy to the `mirador-update` program placed
beside it, and a crates.io copy back to Cargo. The separate `mirador-update`
command remains available for compatibility. A binary installed directly from
an archive has no installation record to follow, so update it manually or run
an installer once.

Updating runs **only when you ask for it** — the normal dashboard never phones
home, unless you set
`[general].check_for_updates = true`, which asks crates.io once a day whether a
newer version exists and says so once in the status bar. That setting is off by
default, sends no identifier, caches its answer, fails silently, and is
overridden by `NO_UPDATE_CHECK=1` or `DO_NOT_TRACK=1` in your environment. If you
installed from source or through another package manager, upgrade the same way
you installed.

From source:

```sh
git clone https://github.com/jchultarsky/mirador
cd mirador
cargo install --path .
```

Or download a pre-built binary from the
[latest release](https://github.com/jchultarsky/mirador/releases/latest).
Every archive carries a `.sha256` beside it, so verify it before running. On
macOS:

```sh
grep . mirador-aarch64-apple-darwin.tar.gz.sha256 | shasum -a 256 -c -
```

and on Linux:

```sh
grep . mirador-x86_64-unknown-linux-gnu.tar.gz.sha256 | sha256sum -c -
```

Either prints `… OK` and exits 0, or prints `FAILED` and exits 1. The `grep .`
drops a trailing blank line that the checksum files are written with; without
it the command still verifies correctly but adds `WARNING: 1 line is
improperly formatted`, and a warning printed next to a checksum is exactly the
wrong thing to make someone squint at.

On Windows, print the hash and compare it with the `.sha256` file beside the
archive:

```powershell
Get-FileHash .\mirador-x86_64-pc-windows-msvc.zip
Get-Content .\mirador-x86_64-pc-windows-msvc.zip.sha256
```

| Platform | Archive |
| --- | --- |
| macOS, Apple silicon | `mirador-aarch64-apple-darwin.tar.gz` |
| macOS, Intel | `mirador-x86_64-apple-darwin.tar.gz` |
| Linux x86-64 | `mirador-x86_64-unknown-linux-gnu.tar.gz` |
| Linux, ARM64 | `mirador-aarch64-unknown-linux-gnu.tar.gz` |
| Windows x86-64 | `mirador-x86_64-pc-windows-msvc.zip` |

Requires Rust 1.95 or newer to build from source.

**NetBSD** (and anywhere pkgsrc runs) has a community package:
`pkgin install mirador`, or build from
[`sysutils/mirador`](https://pkgsrc.se/sysutils/mirador). It is packaged and
maintained in pkgsrc rather than by this repository — the first BSD packaging
of mirador, and the platform whose bug report fixed IPv6-preferring networks
for everyone (1.6.1).

**Windows** has been run and works — installed with the PowerShell one-liner
above, in the default Windows terminal. It is still the least-travelled of the
three: macOS and Linux are used daily and Windows has been checked rather than
lived in, so a report of something off there is genuinely useful. Windows on
ARM is deliberately absent: `ring`, reached through `ureq`'s TLS, does not
build for it.

## Quick start

```sh
mirador
```

On first run it writes a commented config file, lays out every panel, and seeds
the task list and notes with a few examples so nothing starts blank. The
examples are ordinary entries — they explain the keys and then you delete them
with `d`. They are seeded only when there is no file yet, so once you clear
them they stay cleared.

To find the config:

```sh
mirador --config-path
```

Set your city with `L` in the weather panel, or edit `[weather].location`.
Either way you are done: no account, no API key, no environment variables.

### Remembered settings

Some settings you change with a keystroke are remembered across restarts:

| Setting | Key | Panel |
| --- | --- | --- |
| Weather units | `u` | Weather |
| Task sort order | `s` | Tasks |
| Whether completed tasks show | `c` | Tasks |
| Seconds on the clock | `s` | Clock |
| Pomodoro durations | `+` / `-` | Pomodoro |
| Watchlist symbols | `a` / `d` | Markets |
| World clocks | `a` / `e` / `d` | Clock |
| World clock order | `Shift+↑`/`↓` | Clock |
| Weather location | `L` | Weather |
| Agenda file | `f` | Agenda |
| Which panels are shown | `w` | — |
| Where the panels are | `m` | — |
| Panel sizes | `Ctrl+arrows` | — |

They go to three different places, and the split is deliberate.

**Anything to do with `[layout]` is written back into your config** — which
panels exist, and how wide they are. That is the part of the file you actually
read and curate, so a change you make with `w` has to show up there or your
config would end up describing a dashboard nobody is looking at.

The edit is surgical, never a reserialise: mirador rewrites the one line it has
to and leaves your comments, spacing and ordering exactly as they were. Adding
a panel is a one-line diff. If your `[layout]` is formatted in a way it cannot
edit confidently, it says so in the picker and changes nothing rather than
guessing — the failure mode is "that did not stick", not a mangled config.

**Everything else goes to a small state file** beside your tasks, and only what
you actually changed is written. Pressing `u` records the units and nothing
else, so editing any other value in your config still takes effect. Delete that
file and every remembered setting falls back to what the config says; it says
so at the top of itself, because a preference you cannot remember setting needs
an obvious way back.

**And the two editable lists are data, not settings.** Watchlist symbols and
the world-clock list go to files of their own beside your tasks —
`watchlist.toml` and `zones.toml` — because a list you edit from the panel is
something mirador owns, not a preference measured against the config. That is
also why `[stocks].symbols` and the clock zones in the config only seed the
first run: after that, the files are the truth.

### Upgrading

**A new widget will not appear in a config you already have.** A config written
by an earlier version lays out exactly the panels that existed when it was
written, and nothing since. Leaving a panel out is a legitimate choice, so this
cannot be an error and cannot be decided for you — and mirador does not tell
you about it either. It used to, in the status bar and in the help overlay, and
that was a mistake in the same family as an unread badge: a dashboard cannot
tell "hasn't discovered this yet" from "decided against it", and it was
reminding the second group for ever.

Press `w` — a key that is on the status bar and in the help — to see every
widget mirador has and which of them your layout places:

```
╭PANELS────────────────────────────────╮
│    ■ clocks                          │
│    ■ weather                         │
│    ■ todo                            │
│    ■ notes                           │
│    ■ stocks                          │
│    ■ calendar                        │
│    ■ agenda                          │
│  ▸ □ pomodoro                        │
│    ■ watchlog                        │
│    ■ news                            │
│    ■ cpu                             │
│    ■ network                         │
│    ■ calculator                      │
│                                      │
│   written to your config on close    │
│   space toggle   esc close           │
╰──────────────────────────────────────╯
```

`space` toggles, `↑`/`↓` or `j`/`k` move, `esc` closes. A panel appears or
disappears the moment you toggle it, and the change is written to your config
when you close the dialog — as a one-line edit, with your comments untouched.
A new panel joins the row carrying the fewest panels; `m` moves it wherever you
want it afterwards, and `Ctrl+arrows` move the
space around, and that is written too.

The last panel cannot be switched off, because a layout with nothing in it is
rejected at startup and would leave you with a config that will not open.

`Shift+↑` and `Shift+↓` move the whole row the focused panel is in, which is
how you reorder rows rather than move a panel between them. Plain `↑`/`↓` still
moves the panel itself, merging it into the neighbouring row.

`mirador --migrate-config` is still there for keys renamed between versions,
and leaves a backup beside the original.

If a config has gone past fixing, `mirador --reset-config` writes the shipped
defaults over it and copies the old one to `config.toml.bak` first. It also puts
aside the preferences you changed from the keyboard, because those outrank the
config — leaving them would apply your old choices straight back over the file
just restored, and the dashboard would come back looking untouched. Your tasks,
notes and watchlist are left alone, and it says so.

If you want the whole thing back to how it arrived, `mirador --factory-reset`
sets aside everything mirador has written — config, preferences, tasks, notes,
watchlist and world clocks — and the next launch seeds them all again. **Nothing
is deleted.** Every file is renamed to a `.bak` beside itself, so a factory
reset is something you can walk back from with `mv`. Your calendar is not
touched: mirador only ever reads an `.ics`, so that file is yours even when it
sits in mirador's own directory.

Both say what they are about to do and wait for a `y`, and neither runs without
a terminal to ask on unless you add `--yes`. An existing backup is never
overwritten, so resetting twice still leaves the first one recoverable.

## The panels

Thirteen widgets, each answering one question. Put the ones you want in the
layout and drop the rest — a widget your layout leaves out is never built, and
nothing will nag you about it.

### Tasks

The reason this project exists. Other terminal dashboards tend to render a
to-do list as a flat checklist; this one has due dates, priorities, tags,
notes, filtering and full editing without leaving the dashboard.

```
╭┤4 Tasks├───────────────────────────────────────────────────────────┤3 open├╮
│ 1 overdue   1 due today   3 open   by smart                                │
│   DONE PRI TASK                              TAGS                      DUE │
│   [ ]  ▮▮▮ Renew the domain                  #admin            2 days late │
│   [ ]  ▮▮  Reply to the design review        #work                   today │
│   [ ]  ▮   Read the ratatui layout docs      #rust              Fri 07 Aug │
│                                                                            │
│ Card on file expired; the registrar will not auto-renew.                   │
╰────────────────────────────────────────────────────────────────────────────╯
```

The selected task's note sits at the foot of the panel, so a one-line summary
in the list does not mean losing the detail behind it. Priority reads as bars
rather than colour alone, and an overdue task is red *and* counted in the
header — the same fact twice, because colour is the first thing a terminal
theme can take away from you.

`smart` sort is the default and means: unfinished first, then overdue, then by
priority, then by due date. `s` cycles to plain `due`, `priority`, `created`
or `title` when you want something simpler.

### Notes

Free-form notes in a master-detail layout, the shape a mail client uses.

```
╭┤6 Notes├───────────────────────────────┤1├╮
│ 1 note                                    │
│   TITLE                              DATE │
│   Release checklist                ·25 J… │
│ ───────────────────────────────────────── │
│ Release checklist                         │
│ written 22 Jul 2026   edited 25 Jul 2026  │
│ Bump the version, run the four gates, tag │
│ v-something, and let CI build the three   │
╰───────────────────────────────────────────╯
```

The body is visible without pressing anything, which is the whole point: a
note's value is the text inside it, and making you press a key to see any of
it turns "glance at the dashboard" into "operate the dashboard". `/` searches
bodies as well as titles, because the title you wrote in a hurry is often not
what you later search for.

Open a note with `Enter`, then move to its body with `Tab`. `Shift` plus the
arrow keys (or `Home`/`End`) selects text, and `Ctrl+A` selects the whole body.
`Ctrl+C` sends that selection to the terminal clipboard and also keeps an
in-Mirador copy; `Ctrl+V` pastes that copy at the cursor or replaces the next
selection. Typing, Backspace and Delete replace selected text too. With
nothing selected, `Ctrl+C` keeps its ordinary terminal meaning and quits.
Your terminal's normal paste shortcut remains the way to bring outside text
into the note; multiline pastes and literal tabs are kept intact in the body.

**`Ctrl+S` saves, from either field.** `Enter` also saves, but only from the
title, where it cannot be mistaken for a newline — in the body it starts one.
`Esc` cancels and keeps nothing. It is worth knowing which of those you pressed:
trusting the footer and reaching for `Esc` is how a note gets lost.

### Clock and calendar

The first configured zone renders large; the rest become a table showing their
offset *relative to that zone*, which is the number you actually want when
scheduling across one. The calendar prints month grids in the shape `cal`
does, with today marked.

`a` adds a clock and `d` removes the selected one. Adding one opens a list of
cities; type to narrow it, `↑↓` to choose, `Enter` to add.

**Search by the city you mean, not the one in the identifier.** Seattle keeps
time in `America/Los_Angeles`, Bengaluru in `Asia/Kolkata`, Boston in
`America/New_York` — and nobody should have to know that before they can add a
clock. Typing matches the city *or* the identifier, anywhere in either, so
`seattle`, `los_angeles` and `america/` all find the same zone. The city you
picked becomes the clock's label, and the identifier is shown beside it so what
ends up in your config is never a surprise.

Anything not on the list still works: type a zone it does not carry and it is
taken as written, with `HQ = Europe/Berlin` naming it yourself.

`e` reopens that dialog on the selected clock, pre-filled, so fixing a label
does not mean deleting the entry and adding it back. `Shift+↑` and `Shift+↓`
(or `J` and `K`) move the selected clock through the table, and `o` shows where
the file lives.

Like the watchlist, the live list is a data file (`zones.toml`) rather than
config, which is what lets the panel own it; `[clocks].zones` seeds your first
run. The large clock is not removable, and nothing can be moved into its place
either, because a clock panel with nothing to draw large is not a clock panel.

```
╭┤2 Calendar├───────────────────╮
│      July 2026                │
│ Su Mo Tu We Th Fr Sa          │
│           1  2  3  4          │
│  5  6  7  8  9 10 11          │
│ 12 13 14 15 16 17 18          │
│ 19 20 21 22 23 24 25          │
│ 26 27 28 29 30 31             │
╰───── n/p month · t today ─────╯
```

The calendar is deliberately offline — it reads no mail server and no account.
`months` sets how many sit across; a taller panel stacks more rather than
leaving a void, up to a year in view.

### Weather

Current conditions and an hourly forecast from [Open-Meteo](https://open-meteo.com),
which needs no key and no account. `u` switches between imperial and metric at
runtime, converting what is already on screen rather than re-fetching. `L` sets
the location without touching your config; the panel re-geocodes and refetches
on the spot. `r` refetches by hand, ahead of the regular refresh.

A failed refresh keeps the last reading and shows its age in amber rather than
blanking the panel — weather two hours old is still useful, and an empty panel
is not. Columns drop as the panel narrows, in the order that costs you least.

### Markets

A watchlist with the last price, the day's change, and an intraday sparkline.

```
╭┤Markets├────────────────────────────────────────────────────────────────┤7├╮
│   SYMBOL         LAST       CHG        % TODAY                             │
│ ▸ ^GSPC       7413.18     +1.20   +0.02% ▇▄▃▃▂▃▂▁▂▂▃▃                      │
│   ^DJI       52210.08   +262.83   +0.51% ▇▅▃▃▂▂▂▂▂▂▃▄                      │
│   ^IXIC      24932.08    -43.74   -0.18% ▇▄▃▃▂▃▂▂▂▂▄▄                      │
│   ^RUT        2948.03    +18.04   +0.62% ▇▄▃▃▂▃▂▂▃▃▄▄                      │
│   ^NDX       28039.21    -89.13   -0.32% ▇▄▃▃▂▃▂▂▃▂▄▄                      │
│   AAPL         336.91     +3.89   +1.17% ▂▄▅▇▅▅▃▃▂▂▂▄                      │
│   MSFT         389.10     +7.40   +1.94% ▃▄▂▆▃▃▄▅▇▇▆▂                      │
│ via yahoo                                                                  │
╰─────────────────────── a add · d remove · r refresh ───────────────────────╯
```

The default watchlist is the five major US indexes — S&P 500, Dow, Nasdaq
Composite, Russell 2000 and Nasdaq-100 — plus two ordinary stocks, so the panel
shows both kinds on a first run. `^VIX` is the volatility index if you want it,
and `EURUSD=X` and `BTC-USD` work too.

`a` adds a symbol and `d` removes one, and those edits stick — the watchlist
is a data file rather than config, which is what lets the panel own it. See
[Market data](#market-data) before filing a bug about HTTP 429.

### Agenda

What is actually next, read from an `.ics` file on your disk. `f` points it at
one from the panel, with `Tab` completing path segments as you type; a path
that cannot be read is refused there and then rather than after the fact.

```
╭┤5 Agenda├──────────────────────────────┤2 today├╮
│ TODAY                                           │
│   09:15  Standup  ·  Zoom                       │
│ ▸ 14:00  Quarterly planning  ·  Room 12         │
│ TOMORROW                                        │
│   all day Public holiday                        │
│ THURSDAY 30 Jul                                 │
│   09:00  Call with London                       │
╰─────────────────── r reload ────────────────────╯
```

The `▸` marks an event happening now. Days are headed `TODAY` and `TOMORROW`
where that is clearer than a date, and an event you are already in stays on
screen until it ends — the meeting you are late for is the one you most want to
see.

**mirador does not sign in to anything.** No account, no token to refresh, no
background process holding your credentials. Point `[agenda].file` at a calendar
you already have; whatever keeps that file current is your business — an export,
a `vdirsyncer` run, a cron job with `curl`, a symlink into a synced folder. It
is re-read on a timer and when you press `r`.

Until you set `file`, the panel says so and names the path it looked at. Nothing
creates that file: unlike the task list, an empty calendar mirador invented
would be a lie about your day.

Recurring events are expanded for the rules people actually have — daily, weekly
with `BYDAY`, monthly and yearly, with `INTERVAL`, `COUNT`, `UNTIL` and
`EXDATE`. A rule using anything more elaborate, such as `BYSETPOS` or an ordinal
`BYDAY` like `1MO`, shows **only its first occurrence** rather than guessing.
That is deliberate: a calendar that invents a meeting costs you a wasted trip,
where one that misses a repeat costs you a glance at the real thing.

An entry the parser cannot read is counted at the bottom of the panel rather
than dropped silently, so a half-read calendar does not just look empty.

### News

Headlines from RSS feeds you choose.

```
╭┤7 News├──────────────────────────────────────────────╮
│ NASA · 1h old                                        │
│ Astronaut Chris Williams to discuss station mission  │
│                                                      │
│ PHYS.ORG · 22m old                                   │
│ What Australian lakes showed us about Martian        │
│ hydrology                                            │
│                                                      │
│ ARS TECHNICA · 45m old                               │
│ 5th Circuit blocks Texas law requiring websites to   │
│ filter "harmful" speech                              │
╰──── o show link · y copy · ↵ open · r refresh ────────╯
```

**A window, not a feed.** It shows however many stories fit and no more. There
is no scrolling, no count, no unread state, and nothing to dismiss — you glance
at it the way you glance at the weather. That is deliberate and it is the same
reasoning that keeps unread-message counts off this dashboard: a number that
accumulates is a number demanding you zero it.

Stories are interleaved rather than sorted purely by date, so the top of the
panel holds the newest from *each* feed. Sorting by date alone hands the whole
window to whichever outlet publishes most often, which is fresh but is not a
window on the world.

`o` shows the selected story's link, in brass beneath a `↳`, wrapped whole so
you can read all of it.

`y` copies it, by asking your terminal to set the clipboard (OSC 52). No
configuration and no dependency — but it is best-effort: some terminals refuse
the sequence, and tmux needs `set-clipboard on`. mirador cannot tell whether it
worked, which is why it says it sent the link rather than claiming it copied one.

`↵` opens it, and **only if you name a program**:

```toml
[news]
open_command = ["open"]                  # macOS
# open_command = ["xdg-open"]            # Linux
# open_command = ["cmd", "/c", "start"]  # Windows
```

Empty by default, so mirador launches nothing you did not ask for. It is run
directly rather than through a shell, and the link goes on as its own argument,
so nothing in a URL can be read as shell syntax. Same shape as
`[pomodoro].chime_command`: mirador does not pick the program, you name it.

If you would rather select the text with the mouse, note that mirador holds the
mouse — use your terminal's override modifier (Shift in most, Option in macOS
Terminal and iTerm2), or set `[general].mouse = false`.

The shipped feeds are science, space and technology only. Choosing outlets for
general or political news is an editorial act this project has no business
making on your behalf; add your own in `[news.feeds]`. A topic *is* a feed —
RSS has no topic parameter, so subscribing to a science feed is how you ask for
science.

Only the headline, link and date are read. Feed summaries are article prose
belonging to whoever wrote them. Refresh is hourly, and an hour is also the
floor.

### Watch log

What happened while you were not looking.

```
╭┤8 Watch log├──────────────────────────────────╮
│ 14:02  Standup appeared in your calendar      │
│ ──────────────────── since you were here ──── │
│ 13:30  Renew the domain went overdue          │
│ watching from 08:12                           │
╰───────────────────────────────────────────────╯
```

Almost nothing qualifies, and that is the point. The clock, the readings, the
prices and the graphs change continuously and none of it is news; your notes and
tasks change because you changed them. An entry is something that happened *to*
you rather than because of you, and that you would want to know even if you
never looked at the panel it came from. Out of the box that means three things:
the day turning, a task crossing into overdue because of it, and an event
appearing in your `.ics` that you did not put there. An external panel can be
a fourth source: a plugin may hand the log a bounded, *completed* event —
outcomes, not progress — and it appears here under that plugin's id.

The day turning is the one that always happens, which is more useful than it
sounds — it is the divider that tells you where one day's entries end, and it
means the panel has something to say even before you point it at a calendar.

**It has no counter, no unread state, and no effect on any other panel.** Unread
message counts were considered for this dashboard and rejected — a number that
accumulates is a number demanding you zero it. This is a record you consult, not
an inbox that consults you, and nothing in it can be dismissed or acknowledged
because an entry you can dismiss is an entry you are expected to dismiss.

The rule line marks where you were last seen. mirador asks your terminal to
report window focus, which is the only honest signal for "the reader is here" —
every other one measures interaction, and a dashboard you glance at is precisely
one you do not touch. Where the terminal does not report it (and under tmux
unless `focus-events on` is set) it falls back to your last keypress, and where
neither has happened it draws no line at all rather than one in the wrong place.

The log lives in memory and says when it started watching. One written to disk
would come back after a restart with a gap it could not mark — the hours mirador
was not running — and a log that implies a completeness it does not have is
worse than one that admits where it begins.

### Pomodoro

A focus timer, in the same block numerals the clock uses.

| Key | Action |
| --- | --- |
| `space` | Start or pause |
| `n` | Skip to the next phase |
| `r` | Put the current phase back to full |
| `+` / `-` | Lengthen or shorten the phase you are in, by a minute |

Focus intervals are brass and breaks are moss, and the phase is spelled out
above the numerals as well — colour alone is a bad way to tell someone whether
they are meant to be working. A paused timer goes grey rather than blinking:
this is a dashboard you leave open, and a flashing clock is the opposite of
that. Pips along the bottom count the set, and the long break falls when they
fill.

`+` and `-` move the phase length *and* what is left of it together, so adding
a minute eighteen minutes into a focus interval does not rewind you to the
start. Changes last for the session; `[pomodoro]` decides where the timer
starts.

**A chime is available and off by default**, because a dashboard you leave open
all day has no business making noise you did not ask for. Set
`[pomodoro].chime = true` and each phase ends with the terminal bell — which
your terminal is free to render as a sound, a flash, or nothing, according to
settings you have already chosen.

mirador ships no audio library on purpose. Playing a sound file means linking
your platform's audio stack, which on Linux means a C library and its dev
headers on every machine that builds this. If you want a particular sound, name
a player instead and it is run directly, with no shell in the way:

```toml
[pomodoro]
chime = true
chime_command = ["afplay", "/System/Library/Sounds/Glass.aiff"]
```

If that program cannot be started, the panel says so where the durations
usually sit. A notification you believe is working but is not is worse than
none at all.


### Calculator

Type an expression and press Enter. `2 + 3 * 4` is 14, not 20 — precedence is
the arithmetic kind, and brackets work — because what this replaces is `bc` or
`python3 -c`, not a pocket calculator. The answer forms beside the line as you
type; Enter keeps it, and what you have kept feeds up the tape above.

| Key | Action |
| --- | --- |
| `0`–`9` `.` | Type a number |
| `+` `-` `*` `/` | Operators; `x` also multiplies |
| `(` `)` | Group |
| `Enter` or `=` | Keep the answer |
| `c` | Clear the entry |
| `C` | Clear the tape as well |
| `Backspace` | Rub out the last character |
| `y` | Send the selected answer to the clipboard |
| `p` | Paste the selected answer into the live expression |
| `↑` / `↓` | Select an answer on the tape |

An operator typed straight after an answer carries it forward, so `+ 10` after
a result of 57 reads as `57 + 10`. That is what a memory key was for.

The selected tape row stays at full body weight while the others recede. Use
`↑`/`↓` to choose an older answer, then `y` to copy it or `p` to append it to
the expression you are typing. Once the tape has a result, those two actions
replace the typing reminder in the focused panel's border so they remain
discoverable at the default width.

Results are aligned on the decimal point, and the row you are typing is the
brightest thing in the panel with its answer in brass; the tape behind it
recedes. **While this panel has focus, `1`–`9` type digits instead of jumping
to a panel.** It is the only panel that changes what a global key does, and it
is unavoidable — a calculator needs the digits. `Tab` still moves focus, and
`q`, `?`, `w`, `m` and `t` all still work.

An answer too wide for the panel is shown in scientific notation rather than
cut. Everywhere else in mirador a value that will not fit reads as a narrow
terminal; here a number missing its tail is a different number, and there would
be nothing on screen to say so. Narrower still, the working column goes and the
answers stay: an answer without its sum is still an answer.

Nothing is kept when you quit. There is no memory key, no percent key and no
functions — the tape and carrying an answer forward cover what those were for.

### CPU and network

CPU shows average load, a moving history graph and a per-core meter row —
`c` toggles that row when the panel is focused. Network shows receive and
transmit rates as two graphs, plus a session total and the peak rate seen.

Both draw their history in **braille**, which packs two samples into every
character cell and four levels into every row — twice the horizontal
resolution of a block-element sparkline in the same space. The colour gradient
runs *vertically by magnitude*, one colour per row, so the profile stays put
as data scrolls: a graph left on screen all day changes colour when the load
changes, not merely because time passed.

> These two panels are the one thing this README will not show you in a code
> block. Braille is missing from several of the fonts GitHub falls back to, so
> the glyphs render at a width the surrounding box characters do not share and
> the panel tears itself apart. The recording at the top of this file shows
> them as they actually look. It is a real terminal, which is the only place
> the alignment is guaranteed.

Both buffers grow to fill the panel, so `history` is a floor on what is kept
rather than a cap on what a wider panel can draw.

## Command line

Every flag is optional; with none of them mirador opens the dashboard.

| Flag | What it does |
| --- | --- |
| `-c`, `--config <PATH>` | Use a specific config file |
| `--config-path` | Print the resolved config path and exit |
| `--print-config` | Print the default config to stdout and exit |
| `--migrate-config` | Update a config written by an older version |
| `--reset-config` | Replace the config with the defaults, keeping a copy |
| `--factory-reset` | Start over: config, preferences, tasks, notes and watchlist all set aside |
| `--update` | Update through the installer or Cargo and exit |
| `-y`, `--yes` | Do not ask for confirmation |
| `-h`, `--help` | Print help and exit |
| `-V`, `--version` | Print the version and exit |

The config path resolves in that order: `--config` wins over the
`MIRADOR_CONFIG` environment variable, which wins over the platform default —
the same order `--config-path` reports.

Neither reset deletes anything. Every file they touch is renamed to a numbered
`.bak` beside itself, and both list what they will affect before doing it.
They are also separate commands rather than degrees of one: `--reset-config` is
about configuration, `--factory-reset` about everything mirador has written.

## Keys

| Key | Action |
| --- | --- |
| `Tab` / `Shift+Tab` | Move focus between panels |
| `1` – `9` | Jump straight to a panel |
| `?` | Show all key bindings |
| `w` | Choose which panels are shown |
| `m` | Rearrange the panels — see below |
| `t` | Choose a theme, previewing as you move — see below |
| `Ctrl+arrows` | Resize the focused panel against its neighbour |
| `q` / `Ctrl+C` | Quit |

### Rearranging the dashboard

Press `m`, then move the focused panel with the arrow keys. `←` and `→` swap it
with its neighbour; `↑` and `↓` move it between rows, landing it under wherever
it already was rather than at the same index.

**Rows are managed through the panels, not separately.** There is no "add row"
command because there is nothing useful an empty row could do:

- **To open a row**, push a panel past the top or bottom edge. Hold `↑` and the
  panel climbs through the rows; press it once more from the top row and the
  panel takes a row of its own. The height comes out of the row it left, so
  nothing else on screen resizes.
- **To close one**, move its last panel into a neighbour. The row closes behind
  it and hands its height to the row that took the panel.

A panel that already has a row to itself will not take another, so holding `↑`
at the edge stops rather than spawning a row per keypress.

The panels themselves move as you press, so what you see is what you will get.
`Enter` keeps the arrangement and `Esc` puts everything back exactly as it was.
`Tab` picks a different panel to move without leaving the mode, and `Ctrl+arrows`
still resize while you are in there.

A panel keeps the width you gave it when it moves, and the row weights always
add up to what they added up to before — so rearranging one corner of the
dashboard never quietly rescales another.

In the task panel:

| Key | Action |
| --- | --- |
| `j` / `k`, `↑` / `↓` | Move the selection |
| `g` / `G`, `Home` / `End` | Jump to first / last |
| `PageUp` / `PageDown` | Move a screen at a time |
| `Space` | Toggle done |
| `a` / `n` | Add a task |
| `e` / `Enter` | Edit the selected task |
| `d` | Delete (asks first) |
| `p` / `P` | Cycle priority forward / back |
| `s` | Cycle sort mode |
| `c` | Show or hide completed tasks |
| `/` | Filter by title, tag or notes |
| `o` | Show the task file path |

In the add/edit form, `Tab` and `Shift+Tab` move between fields, `Enter` saves
and `Esc` cancels. While a form is open, global keys are suppressed, so typing
a `q` into a task title does not quit the dashboard.

## When something needs you

The status bar carries one line when something will get worse if you ignore it,
and carries nothing otherwise:

```
 ⚠ Quarterly planning in 5m · Room 12
 ⚠ Tasks could not be saved — read-only file system (os error 30)
```

**There is no all-clear.** An indicator saying everything is fine is a light you
have to read in order to learn nothing, and reading it is work. Absence is the
signal.

**It names one thing, never a count**, and it clears itself when the thing
resolves. Nothing to acknowledge, nothing to dismiss.

The bar is deliberately strict about what qualifies: *will this get worse if
nobody acts in the next few minutes*. A save that is failing, an event about to
start, a layout change that did not persist. An overdue task does **not**
qualify — it is notable, the task panel already shows it in red, and a signal lit
for it would be lit permanently. A permanently lit warning is furniture you stop
seeing inside a week, which is the same failure as an unread badge arrived at
from the other direction.

## Due dates

The due field accepts more than ISO dates:

| Input | Meaning |
| --- | --- |
| `2026-07-28` | That date |
| `today`, `tomorrow`, `yesterday` | Relative to now |
| `mon`, `friday`, `sat` | The *next* such weekday, never today |
| `+3d`, `3d`, `2w`, `1m`, `1y` | An offset from today |
| *(empty)* | No due date |

Anything else is rejected with an explanation rather than silently guessed.

## Configuration

One TOML file. Run `mirador --print-config` to see the fully commented default.

The layout is a grid of rows, where `height` and `width` are relative weights
rather than cell counts, so proportions hold at any terminal size:

```toml
[layout]
rows = [
  { height = 30, panels = [
    { widget = "clocks",   width = 26 },
    { widget = "calendar", width = 34 },
    { widget = "weather",  width = 40 },
  ] },
  { height = 45, panels = [
    { widget = "todo",     width = 44 },
    { widget = "notes",    width = 30 },
    { widget = "pomodoro", width = 26 },
  ] },
  { height = 25, panels = [
    { widget = "stocks",  width = 40 },
    { widget = "cpu",     width = 30 },
    { widget = "network", width = 30 },
  ] },
]
```

**Keep that shape if you want `w` and `m` to work.** TOML lets you spell the
same layout as a series of `[[layout.rows]]` sections, and mirador reads it
perfectly well — but it *writes* your config by editing the text, so your
comments survive, and the text it knows how to edit is the form above.
Rearranging a layout written the other way tells you so in the status bar
rather than failing quietly.

### Widgets

| Widget | What it shows |
| --- | --- |
| `clocks` | Any number of world clocks, by IANA timezone |
| `weather` | Current conditions plus an hour-by-hour forecast |
| `todo` | The task list, with full editing |
| `notes` | Free-form notes: a list of titles and dates above the note you are reading |
| `stocks` | A watchlist: last price, the day's change, and an intraday sparkline |
| `calendar` | Month grids in the shape `cal` prints, with today marked |
| `agenda` | What is next, from a local `.ics` file |
| `watchlog` | The watch log: what changed while you were not looking |
| `news` | Headlines from the RSS feeds you choose |
| `pomodoro` | A focus timer: phase, time left, progress, and the set so far |
| `calculator` | Type a sum, press Enter; an adding machine's tape of what you worked out |
| `cpu` | Average utilisation, a moving chart, and per-core meters |
| `network` | Receive and transmit rates as moving charts |

### External panels (optional)

Mirador can host an explicitly configured panel process through its
[versioned JSON-lines protocol](docs/plugin-protocol.md). This is a constrained
customization boundary, not a second built-in widget system: the release binary
embeds no interpreter, discovers nothing, and starts no process unless both its
command is declared and its id is placed in the layout.

```toml
[[plugins]]
id = "example"
command = ["example-mirador-plugin"]

[plugins.config]
plugin_owned_setting = true
```

The host keeps `Ctrl+C`, rendering, clipping, wrapping, input arbitration and
resource bounds on Mirador's side of the pipe. Every such tile is visibly
marked `EXTERNAL`; it may accept focused input, but it cannot draw into the
terminal or block the UI thread. The protocol deliberately offers no PTY or raw
terminal capability.

Plugin commands are trusted code with your permissions, not sandboxed
extensions. Do not put credentials in Mirador's config or command arguments;
a plugin that needs a login collects and owns it inside its panel. Optional
language SDKs remain separate—the experimental Python SDK and example widgets
at [krflol/mirador-plugins](https://github.com/krflol/mirador-plugins) are not a
Python dependency or a Mirador support commitment.

### Market data

The watchlist reads `query1.finance.yahoo.com`, the endpoint Yahoo's own charts
call. **It is not a documented or supported API**, and Yahoo has never published
terms permitting third-party programs to use it. It could be changed, rate-
limited or withdrawn without notice, and whether your particular use of it is
permitted is between you and Yahoo — this project cannot grant you a licence to
someone else's service, and does not claim to.

mirador's position, stated plainly so you can decide for yourself:

- It is used because it is the only source that needs no account and no key, and
  bundling an API key in a public binary would make it a shared secret with a
  shared rate limit.
- It is treated as a guest: one request per symbol, never faster than once a
  minute, sequential rather than parallel, and prices are never written to disk.
- `QuoteSource` is a trait with exactly this in mind. When the endpoint changes
  or you would rather use a provider you have an agreement with, the panel does
  not need rewriting — the source does.
- If none of that sits right with you, leave `stocks` out of your `[layout]`.
  Nothing else in mirador contacts Yahoo, and a panel that is not placed is not
  built.

Two practical consequences, worth knowing before you file a bug:

- **Access is gated on your IP address, not on a key.** Datacenter and VPN
  ranges are blocked wholesale, so the same build can work on your laptop and
  fail on a VPS. Persistent HTTP 429 is that, not a rate you can tune down. The
  data source is a swappable trait for exactly this reason.
- **Polling is deliberately unhurried** — no faster than once a minute, one
  symbol at a time. These services are free and unauthenticated, and a burst of
  parallel requests is what gets an address blocked for everyone behind it.

mirador ships no API key and never will: a key baked into a public binary is a
shared secret, and its rate limit would be shared too. Prices are never written
to disk, only the list of symbols.

### Mouse

Click a panel to focus it, and scroll the wheel over a list or a calendar to
move through it. Scrolling deliberately does *not* move focus, so running the
wheel across the screen cannot take the keyboard away from what you were doing.

While mirador holds the mouse your terminal's own click-to-select-text stops
working; copying a value off the dashboard needs the terminal's override
modifier, which is Shift in most terminals and Option in macOS Terminal and
iTerm2. Set `[general].mouse = false` to keep ordinary selection instead.

### Colours and themes

Every colour the dashboard draws with lives under `[theme]`, and the graph
gradients under `[theme.rx_gradient]` and friends. A colour accepts a name
(`red`, `light-blue`), a hex string (`#ff8800`), or a 256-colour index (`12`).
`reset` uses your terminal's own default.

You can also name a theme instead, and drop the table entirely:

```toml
theme = "high-contrast"
```

**Press `t` to browse them.** The list previews as you move through it, so you
see a theme on your own dashboard before you choose it; `Enter` keeps what is on
screen and `Esc` puts back what you had. Your own themes are listed alongside
the shipped ones. A theme picked this way is remembered, the same way your
weather units and task sort order are — so if you set `theme` in the config and
it appears to be ignored, you picked something else with `t` at some point.

#### What ships

Four are mirador's own:

| Theme | For |
| --- | --- |
| `default` | The shipped look — true colour, dark background |
| `default-light` | Light backgrounds. Not the dark theme inverted: the hues are deepened so they keep their contrast on paper-coloured terminals |
| `high-contrast` | Bright rooms, projectors, low vision. `muted` is deliberately *not* dim — faint grey is the first thing to vanish on a washed-out screen, so de-emphasis is carried by hue instead |
| `ansi` | The sixteen ANSI colours and nothing else, so it follows whatever palette your terminal already uses and works with no true-colour support |

Twelve more are ports of palettes you may already be running in your editor,
terminal or multiplexer, so mirador can match the rest of your screen — seven
dark, five light:

| Theme | Palette |
| --- | --- |
| `nord` | [Nord](https://www.nordtheme.com/docs/colors-and-palettes) — arctic, north-bluish; Frost for the instruments, Aurora for the signals |
| `gruvbox` | [Gruvbox](https://github.com/morhetz/gruvbox) dark — retro groove; the warmest and highest-contrast of the ports |
| `gruvbox-light` | The same retro groove on parchment — the "faded" accents, which are Gruvbox's own light-mode set |
| `dracula` | [Dracula](https://spec.draculatheme.com/) — the loudest, six saturated accents at the same brightness |
| `catppuccin-mocha` | [Catppuccin](https://github.com/catppuccin/catppuccin) Mocha — pastel accents on soft charcoal |
| `catppuccin-latte` | Catppuccin's one light flavour — the same names, rendered darker so they hold on paper |
| `tokyo-night` | [Tokyo Night](https://github.com/enkia/tokyo-night-vscode-theme) Storm — the lifted variant, so mirador's slate chrome stays visible |
| `kanagawa` | [Kanagawa](https://github.com/rebelot/kanagawa.nvim) Wave — Hokusai's ink and pastels; the closest in temperament to mirador's own default |
| `solarized-dark` | [Solarized](https://ethanschoonover.com/solarized/) dark — designed against measured contrast rather than by eye, and the calmest of the ports |
| `solarized-light` | The daylight half of the same design — Solarized shares its accents across both modes and swaps only the greys |
| `everforest-light` | [Everforest](https://github.com/sainnhe/everforest) light — a green-based comfort palette; even its greys lean toward the forest |
| `rose-pine-dawn` | [Rosé Pine](https://rosepinetheme.com/) Dawn — soho vibes at first light; rose for the instruments, pine standing in for green |

These are ports, not reinterpretations: the hex values come from each palette's
own specification, and the theme file cites its source. Body text stays `reset`
in all of them, so it keeps following your terminal's foreground rather than
being pinned to the palette's.

The same key does both jobs, and TOML decides which: a quoted string is a theme
name, a table is colours written out. You cannot do both, because one key cannot
be both types.

#### Writing your own

Put `<name>.toml` in `themes/` beside your config — `mirador --config-path` will
tell you where that is. A file there wins over a bundled theme of the same name,
so you can replace `default` without renaming it, and it appears in the `t` list
marked as yours. The name has to be letters, digits, dashes and underscores;
anything else is not a name mirador can look up, so it is left out of the list.

```toml
# themes/frostbite.toml
inherits = "ansi"

accent         = "frost"
border_focused = "frost"
title          = "frost"

[palette]
frost = "#88c0d0"
```

`inherits` starts from another theme, so a variant is the handful of lines that
differ rather than a copy of all eighteen keys. `[palette]` names colours once,
after which any key can use the name — and redefining a palette entry in a child
theme recolours the keys its *parent* set with it.

**The colour keys have to come before `[palette]`, not after.** In TOML every
key following a table header belongs to that table, so keys written underneath
`[palette]` end up inside it and set nothing. mirador refuses a theme file that
does this and says which keys are affected, because the alternative is a theme
that quietly comes out as the defaults.

### When something is wrong with your config

mirador refuses to start rather than starting wrong. An unknown widget name, a
malformed colour, or a key it does not recognise is reported with a message
that says how to fix it — including, for a key that was renamed in an earlier
version, which key replaced it and that `mirador --migrate-config` will do it
for you.

A key that is silently ignored is the worst outcome a config can have, because
it makes a stale config look like stale code.

## Task file format

Tasks live in one TOML file — by default under your platform's data directory,
or wherever `[todo].file` points. Press `o` in the task panel to see the path —
and the same key answers in every file-backed panel: notes, the watchlist, the
clock zones and the agenda all show theirs.

```toml
[[task]]
id = 1
title = "Renew the domain"
notes = "Registrar sends the reminder to the old address"
due = "2026-08-14"
priority = "high"     # high | medium | low | none
tags = ["admin"]
done = false
created = "2026-08-01"

[[task]]
id = 2
title = "Read the ratatui 0.30 release notes"
priority = "low"
done = true
created = "2026-07-30"
completed = "2026-08-02"   # set when `done` flips; used by the `c` filter
```

Only `id`, `title` and `created` are required; everything else — `done`
included — may be left out.

Writes are atomic — the file is written under a temporary name, flushed to the
disk, and renamed over the original — so an interrupted save or a full disk can
never truncate your tasks. The same goes for your notes, your watchlist and the
config itself.

## Compatibility

**A config or data file that works today will keep working.** That is a promise
about the files mirador reads, and it is worth being precise about what it
covers.

**Your config.** No option that has ever shipped in a released version has been
removed — every key in every `config.toml` written since `0.1.0` is still
accepted, checked against every released version. Options are added, never
renamed out from under you. The four keys that predate `0.1.0` are handled by
`mirador --migrate-config`, which edits the file in place and tells you what it
changed.

Unknown keys in the config are an *error* rather than a warning, on purpose: a
silently ignored setting is a setting you think you have set. If mirador refuses
your config, it names the key and the line.

**Your data files.** `todos.toml`, your notes and `watchlist.toml` have not
changed shape since `0.1.0`. All of them — plus `zones.toml` and `state.toml` —
ignore keys they do not recognise, so a file written by a newer mirador still
opens in an older one with the settings it understands intact. The one exception
is `update-check.toml`, which is a cache: the worst a stale one costs is a single
extra version check.

**The external panel protocol.** From 1.6.0 the promise has a third part:
protocol v1, as written in [docs/plugin-protocol.md](docs/plugin-protocol.md),
is a compatibility commitment. A release that advertises v1 will not remove or
redefine its fields, weaken the host-owned keys, or lower its documented
bounds; an incompatible shape gets a new protocol number rather than a silent
change to this one. A plugin written against v1 today keeps working.

**What this does not promise.** Running an older mirador after a newer one may
drop a preference the older version has no idea about — it cannot write back
what it never understood. Nothing else is lost, and nothing is corrupted.

## Contributing

Bug reports, feature requests and pull requests are welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow, and
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for the ground rules.

Adding a built-in widget means implementing the `Panel` trait, registering the
name in `src/widgets/mod.rs`, and documenting it here. A personal or
language-independent extension can instead implement the
[external panel protocol](docs/plugin-protocol.md) without linking to Mirador.

## Acknowledgements

mirador is a thin layer over other people's hard work. Eleven direct
dependencies, every one of them maintained by someone who did not have to.

**[ratatui](https://ratatui.rs)** deserves top billing. Every frame here is
ratatui: the layout solver, the buffer diffing that makes a redraw cheap
enough to leave running all day, the widget model that made `Panel` a
twenty-line trait instead of a framework. The border-embedded titles and hints
this project leans on are ratatui's block API being more flexible than it had
any obligation to be. It is the successor to
[tui-rs](https://github.com/fdehau/tui-rs) by Florian Dehau, carried forward
by a community that kept it alive rather than letting it archive — and it
brings [crossterm](https://github.com/crossterm-rs/crossterm) with it, which
is what makes the same binary work on macOS, Linux and Windows terminals.

| Crate | What it does here |
| --- | --- |
| [ratatui](https://ratatui.rs) | Every widget, layout and redraw |
| [quick-xml](https://github.com/tafia/quick-xml) | Reading RSS and Atom well enough for a headline |
| [jiff](https://github.com/BurntSushi/jiff) | Dates, IANA timezones, the "2 days late" arithmetic |
| [sysinfo](https://github.com/GuillaumeGomez/sysinfo) | CPU and network counters, per platform |
| [ureq](https://github.com/algesten/ureq) | Blocking HTTP for weather and quotes |
| [serde](https://serde.rs) + [toml](https://github.com/toml-rs/toml) + [serde_json](https://github.com/serde-rs/json) | Config, tasks and notes as hand-editable TOML; JSON from the APIs |
| [anyhow](https://github.com/dtolnay/anyhow) | Error context that survives to the message a user reads |
| [dirs](https://github.com/dirs-dev/dirs-rs) | Finding the right config and data directory per platform |
| [unicode-width](https://github.com/unicode-rs/unicode-width) | Measuring text in display cells, without which every table misaligns |

`unicode-width` is worth singling out: measuring `chars()` instead of display
cells is a bug this project shipped once, and one emoji is enough to slide
every column after it under the wrong header.

### Services

**[Open-Meteo](https://open-meteo.com)** provides the weather, free and
without an API key or an account, which is the only reason the weather panel
can exist in a tool that refuses to bundle credentials. Their
[terms](https://open-meteo.com/en/terms) are generous to open source and they
deserve the traffic; if you build on them at scale, pay them.

Market data comes from Yahoo Finance's public chart endpoint. It is not a
documented API and mirador treats it accordingly — one request per symbol, no
faster than once a minute, never persisted to disk. `QuoteSource` is a trait
so it can be replaced when it inevitably changes.

### Prior art

Techniques were taken deliberately from projects that solved these problems
first, and are described in `CLAUDE.md` where they are implemented:

- **[btop](https://github.com/aristocratos/btop)** and its ancestor
  **[bpytop](https://github.com/aristocratos/bpytop)** — the braille level
  table, and the insight that a gradient should run vertically by magnitude so
  a graph does not shimmer as data scrolls. The CPU and network panels are
  wearing their ideas.
- **[clock-tui](https://github.com/race604/clock-tui)** — run-length row
  encoding for block numerals, which turns thirteen glyphs and free integer
  scaling into about twenty lines.
- **[gitui](https://github.com/gitui-org/gitui)**,
  **[lazygit](https://github.com/jesseduffield/lazygit)** and ratatui's own
  tutorial — focus by dimming the unfocused rather than brightening the
  focused, hints in the bottom border, the jump key in the title.

## License

MIT. See [LICENSE](LICENSE).

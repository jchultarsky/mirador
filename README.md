# mirador

[![CI](https://github.com/jchultarsky/mirador/actions/workflows/ci.yml/badge.svg)](https://github.com/jchultarsky/mirador/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/mirador.svg)](https://crates.io/crates/mirador)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.95+](https://img.shields.io/badge/rust-1.95%2B-dea584.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg)](#install)

An opinionated personal dashboard for your terminal: world clocks, a calendar,
weather, a real task list, notes, a market watchlist, and live CPU and network
charts, laid out how you want them.

It is built for one job — to sit in a terminal tab you leave open all day and
glance at. That single constraint drives every decision below: nothing blinks,
nothing shimmers, and nothing is designed to pull you back to it.

A *mirador* is a lookout — the tower you climb to see everything at once.

```
╭─ Clocks ─────────────────────╮╭─ Weather — Boston, Massachusetts ──────────╮
│ Local   09:41:07  Sat 25 Jul ││ ☀ 74°F  clear                              │
│ UTC     13:41:07  Sat 25 Jul ││ feels 76°F   wind 7 mph   humidity 58%     │
│ London  14:41:07  Sat 25 Jul ││ Sat  ☀   81° /  63°                        │
│ Tokyo   22:41:07  Sat 25 Jul ││ Sun  ⛅   78° /  61°   20%                  │
╰──────────────────────────────╯╰────────────────────────────────────────────╯
┏━ Tasks (4) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ 4 open · 1 overdue · 1 today   sort: smart                                 ┃
┃ ▸ [ ] !!! Renew the domain               #admin            2 days late     ┃
┃   [ ] !!  Reply to the design review     #work             today           ┃
┃   [ ] !   Publish 0.0.0 placeholder      #mirador #rust    in 3 days       ┃
┃   [ ]     Read the ratatui layout docs                     Fri 07 Aug      ┃
┃ Reserve the crates.io name before someone else does.                       ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
╭─ CPU (10 cores) ─────────────╮╭─ Network ──────────────────────────────────╮
│  12.4%  average   120s window││ ↓    1.2 MB/s    ↑    184 KB/s             │
│ ▁▂▃▂▁▁▂▅▇▆▃▂▁▁▂▂▁▁▁▂▃▂▁▁▁▂▁▁ ││ ▁▁▂▁▁▃▇█▅▂▁▁▁▂▁▁▁▁▂▁▁▁▁▁▂▁▁▁▁▁▁▁▁▁▂▁▁▁▁▁▁ │
╰──────────────────────────────╯╰────────────────────────────────────────────╯
```

## Why

There are good terminal dashboards already, but the task panels in them tend to
be flat checklists. mirador treats the to-do list as a first-class feature: due
dates, priorities, notes, tags, and full editing without leaving the dashboard.
Everything is stored in a plain TOML file you can hand-edit or keep in git.

No accounts. No API keys. No telemetry. Weather comes from
[Open-Meteo](https://open-meteo.com), which is free and key-less.

## Install

From source, which is the way to get the current build:

```sh
git clone https://github.com/jchultarsky/mirador
cd mirador
cargo install --path .
```

Requires Rust 1.95 or newer. Tested on macOS and Linux; Windows is untested
rather than unsupported.

> **On crates.io, the published version is `0.0.0`** — a reservation of the
> name, not a release. `cargo install mirador` works and gives you a running
> dashboard, but it will not track `main` until `0.1.0` ships. Build from
> source until then. There are no pre-built binaries yet.

## Quick start

```sh
mirador
```

On first run it writes a commented config file and starts with a sensible
default layout. To find the config:

```sh
mirador --config-path
```

Edit `[weather].location` to your own city and you are done.

## Keys

| Key | Action |
| --- | --- |
| `Tab` / `Shift+Tab` | Move focus between panels |
| `1` – `9` | Jump straight to a panel |
| `?` | Show all key bindings |
| `q` / `Ctrl+C` | Quit |

In the task panel:

| Key | Action |
| --- | --- |
| `j` / `k`, `↑` / `↓` | Move the selection |
| `g` / `G` | Jump to first / last |
| `Space` | Toggle done |
| `a` | Add a task |
| `e` | Edit the selected task |
| `d` | Delete (asks first) |
| `p` / `P` | Cycle priority forward / back |
| `s` | Cycle sort mode |
| `c` | Show or hide completed tasks |
| `/` | Filter by title, tag or notes |
| `o` | Show the task file path |

In the add/edit form, `Tab` and `Shift+Tab` move between fields, `Enter` saves
and `Esc` cancels. While a form is open, global keys are suppressed, so typing
a `q` into a task title does not quit the dashboard.

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
    { widget = "todo",  width = 58 },
    { widget = "notes", width = 42 },
  ] },
  { height = 25, panels = [
    { widget = "stocks",  width = 40 },
    { widget = "cpu",     width = 30 },
    { widget = "network", width = 30 },
  ] },
]
```

### Widgets

| Widget | What it shows |
| --- | --- |
| `clocks` | Any number of world clocks, by IANA timezone |
| `weather` | Current conditions plus a 1–7 day forecast |
| `todo` | The task list, with full editing |
| `notes` | Free-form notes: a list of titles and dates above the note you are reading |
| `stocks` | A watchlist: last price, the day's change, and an intraday sparkline |
| `calendar` | Month grids in the shape `cal` prints, with today marked |
| `cpu` | Average utilisation, a moving chart, and per-core meters |
| `network` | Receive and transmit rates as moving charts |

### Market data

The watchlist reads a free, keyless endpoint. Two things follow from that, and
they are worth knowing before you file a bug:

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

Colours accept a name (`red`, `light-blue`), a hex string (`#ff8800`), or a
256-colour index (`12`). `reset` uses your terminal's own default.

An unknown widget name or a malformed colour is reported at startup with a
message that says how to fix it, rather than being ignored.

## Task file format

Tasks live in one TOML file — by default under your platform's data directory,
or wherever `[todo].file` points. Press `o` in the task panel to see the path.

```toml
[[task]]
id = 1
title = "Publish 0.0.0 placeholder"
notes = "Reserve the crates.io name"
due = "2026-07-28"
priority = "high"     # high | medium | low | none
tags = ["mirador", "rust"]
done = false
created = "2026-07-25"
```

Writes are atomic — mirador writes a temporary file and renames it — so an
interrupted save can never truncate your tasks.

## Contributing

Bug reports, feature requests and pull requests are welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow, and
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for the ground rules.

Adding a widget means implementing the `Panel` trait, registering the name in
`src/widgets/mod.rs`, and documenting it here. Nothing else in the codebase
needs to know it exists.

## Acknowledgements

Built with [ratatui](https://ratatui.rs). Weather data from
[Open-Meteo](https://open-meteo.com). Dates and timezones handled by
[jiff](https://github.com/BurntSushi/jiff); system metrics by
[sysinfo](https://github.com/GuillaumeGomez/sysinfo).

## License

MIT. See [LICENSE](LICENSE).

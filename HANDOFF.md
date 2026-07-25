# HANDOFF — picking mirador up on another machine

Written 25 July 2026, moving from a work laptop to a home laptop.

`CLAUDE.md` is the durable context: architecture, invariants, the visual system,
settled product decisions. **Read that first.** This file is the perishable
half — where the work stopped, what is decided, and what is waiting on you.
Delete it once it stops being true.

## Get running

```sh
git clone https://github.com/jchultarsky/mirador.git
cd mirador
cargo run
```

First run writes a commented config and starts with a working layout. Nothing
else is needed: no accounts, no API keys, no environment variables.

## The gate before every commit

All four must be silent. Running only the first three is how a red build reached
`main` — rustdoc rejects a link to a `cfg(test)` item and nothing else notices.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items
```

Two traps that cost real time, both worth knowing before you hit them:

- **`assets/default_config.toml` is `include_str!`-baked.** Editing it does
  nothing until you rebuild. Twice a change looked like it had not landed when
  it simply had not been compiled.
- **Bumping `rust-version` is never only a metadata edit.** Clippy gates lints
  on it. Going 1.85 → 1.88 turned every nested `if let` into a `collapsible_if`
  error; 1.88 → 1.95 turned `Duration::from_secs(3600)` into a lint. Expect a
  round of mechanical fixes with any bump.

## State of the repo

Thirteen commits on `main`, CI green on all six jobs as of `c46d84c`.

Shipped this session, in rough order: mouse support; a calendar panel; the
notes panel; the stock watchlist; panel resizing; the typography change from
letterspaced to bold labels; the layout allocator that lets a panel hand its
surplus space to a neighbour; weather staleness handling and unit switching.

Four bugs found and fixed along the way, listed because the *class* of each is
likely to recur:

| Bug | Class |
| --- | --- |
| Grid rows indexed cells by surviving column, so a dropped column slid every later value under the wrong header | one index used for two different orderings |
| Watchlist sparkline vanished when the panel width was capped | the same constant computed by hand in three places |
| Task ids reused after a deletion, so a stale id could act on a different task | `max(id) + 1` instead of a high-water mark |
| CPU and network graphs stopped short of a wide panel's left edge | a config count silently capping what a bigger panel can show |

## Waiting on you

**1. Repo description and topics are still unset.** I had write access but not
admin, and repo metadata is an owner-level setting. From the `jchultarsky`
account:

```sh
gh repo edit jchultarsky/mirador --description "A calm terminal dashboard: world clocks, calendar, weather, tasks, notes, a stock watchlist, and live CPU and network charts, in a config-driven grid." --add-topic rust --add-topic tui --add-topic ratatui --add-topic terminal --add-topic dashboard --add-topic cli --add-topic terminal-dashboard --add-topic todo --add-topic weather --add-topic system-monitor --add-topic productivity
```

The About panel on the repo page does the same thing without the CLI. Leave the
homepage empty until the crates.io name exists, then point it there.

**2. The crates.io name is not reserved.** Reservation is first-come with no
reclamation, and `mirador` is an ordinary English word. Publishing a `0.0.0`
placeholder is cheap insurance and the one item here with a deadline attached.

**3. Settings editable from the UI — a decision, not a task.** mirador
deliberately never rewrites its config, so an edited setting has nowhere to go.
The watchlist solved this by treating symbols as *data* in their own file, and
that worked well. Whether every setting follows suit, or the never-rewrite
stance changes, is a product call. Do not pick one silently; it shows.

**4. The wiki is enabled and empty.** With a thorough README and CONTRIBUTING
in-repo it will only become a second stale place to look. `--enable-wiki=false`
unless you have plans for it.

## Notes for whoever works on this next

- **Drive it in a real terminal, not just `TestBackend`.** Run it under `tmux`
  and read the output with `tmux capture-pane -p`. Several layout bugs this
  session were invisible to the test backend because nothing errors when
  content is pushed off the bottom. `-e` keeps the escape sequences, which is
  the only way to prove a colour bug — a screenshot of black-on-black shows
  nothing either way.
- **The four questions the dashboard exists to answer** are in `CLAUDE.md` and
  they are the filter for new panels. A panel that answers none of them needs a
  better reason than "it would be easy".
- **A test that cannot fail is documentation with a `#[test]` on it.** The id
  reuse bug shipped with a test named `ids_are_unique_and_survive_deletion`
  sitting right on top of it: the test compared the new id against the
  *surviving* task rather than the *removed* one, so it passed throughout. When
  you fix a bug that had a test nearby, check the test would have caught it —
  break the fix on purpose and watch it go red. Twice this session that check
  was the difference between a real test and a reassuring one.

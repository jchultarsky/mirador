# Contributing to mirador

Thanks for your interest. Bug reports, feature requests and pull requests are
all welcome.

## Before you start

For anything larger than a bug fix, please open an issue first so we can agree
on the approach. It is disappointing for everyone when a finished pull request
turns out to be heading somewhere the project is not going.

Small fixes — a typo, an off-by-one, a missing edge case — need no discussion.
Just send the pull request.

## Development

```sh
git clone https://github.com/jchultarsky/mirador
cd mirador
cargo test
cargo run
```

Requires Rust 1.95 or newer.

Before pushing, run what CI runs. All six, not the first three — the last three
catch things the others cannot, and each has reddened `main` before: a broken
intra-doc link only `cargo doc` sees, an `exclude` in `Cargo.toml` that dropped
a file the build needed, and a dependency `cargo deny` refused.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items
cargo deny check
cargo publish --dry-run
```

`--locked` matters and is not decoration: without it cargo will quietly rewrite
`Cargo.lock` to satisfy a build, so a local run can pass against a dependency
set the repository does not contain. CI passes it everywhere for that reason.

CI also builds and tests on macOS **and Windows**, and type-checks against the
minimum supported Rust version:

```sh
cargo +1.95.0 check --locked --all-targets
```

Windows is in that matrix because a Windows-only startup panic once sat in
`main`, and more recently because a test spawned a command that does not exist
there — found by a contributor running the suite on their own machine, not by
CI, whose runner image happened to provide it.

Note that `clippy` gates some lints on the `rust-version` field, so a newer
toolchain locally can report warnings CI does not, and vice versa. When the two
disagree, CI is the one that matters.

To try changes against a throwaway config instead of your real one:

```sh
mirador --print-config > /tmp/mirador.toml
cargo run -- --config /tmp/mirador.toml
```

## Adding a widget

The `Panel` trait in `src/panel.rs` is the seam. It is in-tree — mirador has no
library target and `widgets::build` dispatches on a fixed match — so adding a
widget means a pull request rather than a separate crate. Six places to touch:

1. Create `src/widgets/<name>.rs` and implement `Panel`.
2. Add a config struct to `src/config/widgets.rs`, add the type to the
   `pub use widgets::{…}` list in `src/config/mod.rs`, and add a field on
   `Config` in the same file. All three: the field names the type unqualified,
   so without the re-export it does not compile. Give every value a `Default`,
   and mark the struct `#[serde(default, deny_unknown_fields)]` — a config key
   that is silently ignored makes a stale config look like stale code.
3. Add the name to `WIDGET_NAMES` and an arm to `build` in
   `src/widgets/mod.rs`.
4. **If your widget draws through `crate::grid::Grid`**, add
   `("<name>", crate::widgets::<name>::COLUMNS)` to `EVERY_GRID` in
   `src/grid.rs`. A test walks the tree for `const COLUMNS` declarations and
   fails if the registry disagrees, because a grid that is not listed is a grid
   the overflow sweep never checks — and an unchecked grid draws rows wider than
   its panel, which the terminal then cuts without saying so.
5. Document the widget in `assets/default_config.toml` and in the README's
   widget table.
6. Add tests for the logic that is not drawing — parsing, formatting,
   thresholds, state transitions.

Steps 2 and 4 are the ones that bite. The calculator is the proof: the pull
request that added the panel touched no `src/grid.rs` at all, and a later one
had to add the registry entry when the panel gained a grid.

Widgets must not block. If your widget needs network or disk I/O, do it on a
background thread and poll the result in `tick`, as `weather.rs` does. A panel
that blocks freezes the whole dashboard.

If your widget needs a setting the user can change from the keyboard, two
things already exist for it. A value that is free text — a path, a place, a
name — gets a `Prompt` from `src/prompt.rs`: open it with the current value,
validate the answer yourself, and call `reject` to keep the dialog open with
the text still in it. Return the value from `Panel::remember` and it persists
through `state.rs`, which records only what differs from the config so the
setting can be un-set again.

A *list* the user can add to and remove from is different, and does not belong
in the config at all: mirador never rewrites that file, so a list kept there
could only be changed in an editor. Give it a data file beside the tasks, the
way `quote.rs` does for the watchlist and `zones.rs` for the world clocks, and
let the config seed the first run only.

While a prompt is open your panel must return `true` from
`Panel::captures_input`, or the first `q` someone types into it will quit the
dashboard.

## Adding a theme

Write `assets/themes/<name>.toml` and add it to `BUNDLED` in `src/themes.rs`.
The `t` picker then lists it — that part needs nothing else. Two prose counts
do have to follow, though, and nothing pins them: "Ten ship inside the binary"
in `assets/default_config.toml`, and the theme table in the README.

Four things will catch you out. The first three have tests rather than
conventions behind them:

1. **Colour keys go *before* `[palette]` and before the gradient tables.** TOML
   assigns every key after a table header to that table, so a colour written
   below `[palette]` lands inside it and silently sets nothing.
2. **Set every key in `Theme::KEYS`** unless the theme `inherits` another.
3. **The filename must be `[A-Za-z0-9_-]+`.** A theme is looked up by name, not
   by path, so anything else cannot be loaded.
4. **`text` stays `reset` in a ported palette.** Body text follows the reader's
   own terminal foreground; pinning it to your palette breaks on the half of
   terminals not configured the way yours is. This one is a convention rather
   than a test, and `high-contrast` is the deliberate exception — it sets
   `text = "white"`, because guaranteed contrast is the entire point of that
   theme and deferring to the terminal would defeat it.

Keep `border` and `muted` distinct, or secondary text ends up as dim as the
chrome and reads as broken rather than de-emphasised.

If you are porting a palette from elsewhere, take the hex values from its own
specification and cite the source in a comment at the top of the file. Adjust
how the palette maps onto mirador's keys as much as you like; do not adjust the
palette. Someone choosing `nord` wants Nord.

## Code standards

- `cargo clippy --all-targets -- -D warnings` must pass. The crate enables
  `clippy::pedantic`; if a lint is genuinely wrong for a piece of code, add a
  targeted `#[allow]` with a comment explaining why, rather than widening the
  allow list in `Cargo.toml`.
- `unsafe` is forbidden crate-wide.
- Comments explain *why*, not *what*. Do not narrate code that speaks for
  itself; do explain a non-obvious constraint, a workaround, or an ordering
  that matters.
- Error messages should tell the user how to fix the problem, not just what
  went wrong. Compare "unknown widget `nope`. Available widgets: …" against
  "invalid configuration".

## Tests

Test behaviour that could plausibly break, and prefer tests that would fail for
a real reason over tests that restate the implementation.

Areas worth covering: parsing and validation, date arithmetic across month and
year boundaries, multi-byte text handling, ordering and sorting rules, bounds
on buffers and selections, and round-tripping data through disk. The task panel
is tested by driving the same key events the terminal sends, which is usually
the clearest way to test panel behaviour.

Drawing *is* tested, and some of the strongest guards here are render tests:
`no_grid_in_the_program_ever_overflows_its_width`,
`every_widget_renders_at_any_size_without_panicking`, and the sweeps that check
no panel draws a value the terminal will cut in half. Render into ratatui's
`TestBackend` and assert on what reached the cells rather than on internal
state — a buffer is the only place some of these faults are visible.

Keep drawing code thin anyway, so the logic worth testing can also be tested
without a terminal.

## Commit messages

Write a short imperative subject line ("Fix off-by-one in the sparkline
window"), and use the body to explain why the change is needed if it is not
obvious. Reference the issue number when there is one.

## Releasing

Maintainers only:

1. Update `CHANGELOG.md`, moving items out of `Unreleased` into a new version
   section with a date — **and add the link definition at the foot of the file**
   alongside the others, then repoint `[Unreleased]` at the new tag. Seven
   releases in a row skipped this and rendered as plain text instead of compare
   links; `every_released_version_has_a_changelog_link` now fails if you do.
2. Bump `version` in `Cargo.toml`, and the released-version line in
   `CLAUDE.md` — a test compares the two.
3. `cargo publish --dry-run` to check the packaged crate.
4. **Open a pull request for the bump and merge it.** `main` is protected, so
   the version commit arrives as a squash.
5. **Tag the squashed commit on `main`**, not your local one. Tagging before the
   merge appears to work — the tag pushes, the release workflow runs, the
   artifacts are right — and then the squash orphans the commit underneath it,
   so `git describe` on `main` cannot see the release.
6. `cargo publish`. The release workflow builds and uploads the GitHub release
   artifacts; it does **not** publish the crate. That step is manual and comes
   after the tag.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
Participating means agreeing to uphold it.

## License

By contributing, you agree that your contributions will be licensed under the
MIT License that covers this project.

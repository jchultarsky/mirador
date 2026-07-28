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

Before pushing, run what CI runs. All six, not the first three — the last two
catch things the others cannot, and both have reddened `main` before: a broken
intra-doc link only `cargo doc` sees, and an `exclude` in `Cargo.toml` that
dropped a file the build needed.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items
cargo publish --dry-run
```

CI also builds and tests on macOS, and type-checks against the minimum
supported Rust version:

```sh
cargo +1.95.0 check --all-targets
```

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
widget means a pull request rather than a separate crate. Five places to touch:

1. Create `src/widgets/<name>.rs` and implement `Panel`.
2. Add a config struct to `src/config/widgets.rs` and a field on `Config` in
   `src/config/mod.rs`, with a `Default` implementation for every value. Mark
   the struct `#[serde(default, deny_unknown_fields)]` — a config key that is
   silently ignored makes a stale config look like stale code.
3. Add the name to `WIDGET_NAMES` and an arm to `build` in
   `src/widgets/mod.rs`.
4. Document the widget in `assets/default_config.toml` and in the README's
   widget table.
5. Add tests for the logic that is not drawing — parsing, formatting,
   thresholds, state transitions.

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
That is the whole change — the `t` picker lists whatever is in `BUNDLED`.

Four things will catch you out, and each has a test rather than a convention:

1. **Colour keys go *before* `[palette]` and before the gradient tables.** TOML
   assigns every key after a table header to that table, so a colour written
   below `[palette]` lands inside it and silently sets nothing.
2. **Set every key in `Theme::KEYS`** unless the theme `inherits` another.
3. **The filename must be `[A-Za-z0-9_-]+`.** A theme is looked up by name, not
   by path, so anything else cannot be loaded.
4. **`text` stays `reset`.** Body text follows the reader's own terminal
   foreground; pinning it to your palette breaks on the half of terminals not
   configured the way yours is.

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

Rendering itself is not unit tested. Keep drawing code thin so that the logic
worth testing lives outside it.

## Commit messages

Write a short imperative subject line ("Fix off-by-one in the sparkline
window"), and use the body to explain why the change is needed if it is not
obvious. Reference the issue number when there is one.

## Releasing

Maintainers only:

1. Update `CHANGELOG.md`, moving items out of `Unreleased` into a new version
   section with a date.
2. Bump `version` in `Cargo.toml`.
3. `cargo publish --dry-run` to check the packaged crate.
4. Tag `vX.Y.Z` and push the tag.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
Participating means agreeing to uphold it.

## License

By contributing, you agree that your contributions will be licensed under the
MIT License that covers this project.

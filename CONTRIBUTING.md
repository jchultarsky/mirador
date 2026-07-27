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

The `Panel` trait in `src/panel.rs` is the extension seam. To add a widget:

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

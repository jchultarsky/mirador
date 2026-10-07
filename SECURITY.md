# Security Policy

## Supported versions

Only the latest release receives fixes; there are no backports. In practice
that means whatever is on the
[releases page](https://github.com/jchultarsky/mirador/releases) right now.

<!-- No version number in this table, on purpose: it said "0.7.x" from 0.7.1
     until 1.19.2, long after 1.0, because nothing made anyone revisit it. -->

| Version | Supported |
| --- | --- |
| The latest release | Yes |
| Anything older | No — upgrade |

## Reporting a vulnerability

Please do not open a public issue for a security problem.

Report it privately through GitHub's
[security advisory form](https://github.com/jchultarsky/mirador/security/advisories/new),
or by email to jchultarsky@gmail.com.

Please include what the problem is, how to reproduce it, and what an attacker
could achieve. A proof of concept helps but is not required.

You can expect an acknowledgement within a week. This is a spare-time project,
so please be patient with the timeline for a fix; you will be kept informed
either way, and credited in the advisory unless you prefer otherwise.

## Scope

mirador is a local terminal application. It opens no listening ports,
requires no credentials, and stores no secrets.

### What it reads and writes

- **Its own files**, which it reads and writes: the configuration, which it
  edits in place when you rearrange the dashboard, reset its keys or run
  `--migrate-config`; the task, note, watchlist and clock-zone files; the
  remembered settings in `state.toml`; and the update-check cache. The
  `--reset-config` and `--factory-reset` flags rename these aside as
  numbered `.bak` files rather than deleting them.
- **Theme files** in a `themes` folder beside the configuration, read only.
- **A calendar `.ics` file** for the agenda panel: `calendar.ics` in
  mirador's data directory, or the file `[agenda].file` names. mirador only
  ever reads it, but you may have got it from anyone.
- **System metrics**, through [sysinfo](https://crates.io/crates/sysinfo) and,
  for the battery panel,
  [starship-battery](https://crates.io/crates/starship-battery).

### Where it connects

Outbound only, and only for the panels you place or the options you turn on.
Every address mirador ships with is HTTPS; a feed you add is fetched at the
address you give it, and a redirect from any of these hosts is followed.

- `api.open-meteo.com` and `geocoding-api.open-meteo.com` —
  [Open-Meteo](https://open-meteo.com), for the weather panel. It sends the
  configured location, or the configured coordinates.
- `query1.finance.yahoo.com` — for the stocks panel. It sends the ticker
  symbols on your watchlist, with a browser user agent because Yahoo refuses
  anything else. This is Yahoo's undocumented chart endpoint; see
  [Market data](README.md#market-data) for what that means for you.
- **Every feed in `[news].feeds`**, for the news panel. The shipped config
  names three: `www.nasa.gov`, `phys.org` and `feeds.arstechnica.com`. The
  news panel is in the default layout, so a default install contacts them.
- `crates.io`, only if you set `[general].check_for_updates = true`.
- When you run `mirador --update`: the updater the installer put beside
  mirador, which downloads from GitHub, or `cargo install`, which downloads
  from crates.io.

None of these requests carries an identifier, a credential, or anything else
about the machine. The weather and update-check requests name mirador and its
version in the user agent.

### What it runs

mirador starts a program only when your configuration names it or you run
`mirador --update`, and always directly rather than through a shell. On
Windows, a program that is a `.bat` or `.cmd` script is run by `cmd.exe`,
which is how Windows runs scripts; Rust escapes its arguments for that.

- `[pomodoro].chime_command`, at the end of a phase when `chime = true`.
- `[news].open_command`, when you press Enter on a headline, with the link as
  its last argument. mirador hands it only `http` and `https` links, and
  refuses to hand one to `cmd` or PowerShell, which would read the link as a
  command line.
- Each `[[plugins]]` entry: an external panel running as its own process and
  talking to mirador over the protocol in
  [docs/plugin-protocol.md](docs/plugin-protocol.md). A plugin runs with your
  permissions and may contact anything. mirador bounds and checks what a
  plugin sends back to it, but it is not a sandbox.
- `mirador --update` runs the updater the installer put beside mirador, or
  asks `cargo` whether it installed this copy and, if so, runs
  `cargo install mirador --locked`. Only when you run that flag.

### Worth reporting

- Anything that lets a crafted task, note, watchlist or zone file, calendar,
  RSS feed, or plugin message cause code execution, or a write to anything
  but mirador's own files
- Anything that lets a hostile HTTP response from any host above, a hostile
  RSS feed or a hostile `.ics` file do more than show a wrong or missing
  reading — a crash, a hang, unbounded memory, or text that reaches the
  terminal as a control sequence all count
- Anything that causes mirador to send data it should not, to any host above
  or anywhere else
- A dependency advisory that materially affects mirador as it is used here
- A gap in the release pipeline beyond the ones already disclosed below

### Out of scope

- Anything done by a program your configuration names — a chime, an opener,
  a plugin. The configuration is trusted: naming a program there is how you
  ask mirador to run it, and whoever can write it can already run code as you.
- A malicious config or theme file causing a crash or a bad layout, for the
  same reason
- Denial of service through absurd config values, such as a several-million
  sample history buffer
- Vulnerabilities in Open-Meteo, Yahoo, a feed's host, GitHub or crates.io
  themselves

## Verifying a download

Every release artifact carries a [GitHub build
provenance](https://docs.github.com/actions/security-guides/using-artifact-attestations-to-establish-provenance-for-builds)
attestation, signed by GitHub's Sigstore instance. This is the check worth
running, because it proves more than a checksum does — not just that the bytes
are intact, but that they were built by this repository's release workflow from
a specific commit:

```
gh attestation verify --repo jchultarsky/mirador mirador-aarch64-apple-darwin.tar.gz
```

Each archive also has a `.sha256` sibling, and the release carries a combined
`sha256.sum`.

### What the installers do not check

The one-line installers come from [dist](https://axodotdev.github.io/cargo-dist/)
and their verification is dist's, not ours. As of dist 0.32.0:

- The shell installer verifies the sha256 of the archive it downloads, but
  **not** of the `mirador-<triple>-update` updater binary it fetches alongside
  it. `sha256.sum` does not list the updater either.
- The PowerShell installer verifies **nothing**. It contains no hash check at
  all.

In both cases the only thing standing between you and a substituted binary is
TLS to `github.com` — which is also true of the archive's checksum, since the
checksum is served from the same release over the same connection. So this is a
defence-in-depth gap rather than an open door, and it is the reason attestations
are the recommended check above.

If that trade is not one you want to make, install from source instead:

```
cargo install mirador --locked
```

This is disclosed rather than fixed because the installers are generated, and
patching generated files by hand is worse than documenting them accurately.

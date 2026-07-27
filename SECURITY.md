# Security Policy

## Supported versions

mirador is pre-1.0. Only the latest released minor version receives fixes;
there are no backports. In practice that means whatever is on the
[releases page](https://github.com/jchultarsky/mirador/releases) right now.

| Version | Supported |
| --- | --- |
| 0.6.x | Yes |
| < 0.6 | No — upgrade |

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

mirador is a local terminal application. It opens no listening ports, requires
no credentials, and stores no secrets.

It reads its own configuration, task, note and watchlist files, samples system
metrics through [sysinfo](https://crates.io/crates/sysinfo), and makes outbound
HTTPS requests to exactly two hosts, both only when the corresponding widget is
enabled:

- `api.open-meteo.com` and `geocoding-api.open-meteo.com` —
  [Open-Meteo](https://open-meteo.com), for the weather panel. It sends the
  configured location, or the configured coordinates.
- `query1.finance.yahoo.com` — for the stocks panel. It sends the ticker
  symbols on your watchlist. This is Yahoo's undocumented chart endpoint; see
  [Market data](README.md#market-data) for what that means for you.

Neither request carries an identifier, a credential, or anything else about the
machine.

Things worth reporting:

- Anything that lets a crafted config, task, note or watchlist file cause code
  execution, or write outside the configured paths
- Anything that causes mirador to send data it should not, to either host above
  or anywhere else
- Anything that lets a hostile HTTP response from either host affect the machine
  beyond a wrong reading on screen
- A dependency advisory that materially affects mirador as it is used here
- A gap in the release pipeline beyond the ones already disclosed below

Things that are out of scope:

- A malicious config file causing a crash or a bad layout. mirador trusts its
  own config file; if an attacker can write to it, they can already run code as
  you.
- Denial of service through absurd config values, such as a several-million
  sample history buffer
- Vulnerabilities in Open-Meteo or Yahoo themselves

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

The one-line installers come from [dist](https://opensource.axo.dev/cargo-dist/)
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

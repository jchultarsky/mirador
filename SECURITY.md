# Security Policy

## Supported versions

mirador is pre-1.0. Only the latest released version receives fixes.

| Version | Supported |
| --- | --- |
| 0.1.x | Yes |

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

mirador is a local terminal application. It reads its own configuration and
task files, samples system metrics, and makes outbound HTTPS requests to
[Open-Meteo](https://open-meteo.com) for weather. It opens no listening ports
and requires no credentials.

Things worth reporting:

- Anything that lets a crafted config or task file cause code execution, or
  write outside the configured paths
- Anything that causes mirador to send data it should not, to Open-Meteo or
  anywhere else
- A dependency advisory that materially affects mirador as it is used here

Things that are out of scope:

- A malicious config file causing a crash or a bad layout. mirador trusts its
  own config file; if an attacker can write to it, they can already run code as
  you.
- Denial of service through absurd config values, such as a several-million
  sample history buffer
- Vulnerabilities in Open-Meteo itself

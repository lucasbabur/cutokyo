# Security policy

## Supported versions

Cutokyo is pre-1.0 and has no released supported version yet. Security fixes are made
on the current development line. This file will gain a release support table before
the first public package is published.

## Reporting a vulnerability

Do not open a public issue containing an exploit, credential, transcript, provider
header, local path, or other sensitive evidence. Use GitHub's private vulnerability
reporting for `lucasbabur/cutokyo`. If that option is unavailable, open a minimal
public issue asking a maintainer to establish a private channel, without technical
vulnerability details.

Include the affected revision, platform, impact, a minimal synthetic reproduction,
and whether the issue crosses the local-only, secret-redaction, plugin, proxy,
configuration, or database boundary. Never test against another person's data or
provider account.

Maintainers will acknowledge a usable report when capacity permits, coordinate a fix
and disclosure, and credit reporters who request credit. Because there is no release
yet, no response-time or bounty promise is made.

## Security boundaries

v0.x is local-only and defaults to no telemetry. It does not provide application-level
database encryption or claim portable plugin filesystem/network sandboxing. Use
full-disk encryption and review local plugins. Proxy capture and AI analysis are
explicit opt-in egress paths. These disclosed limits are not vulnerabilities by
themselves; bypassing consent, leaking secrets, corrupting local evidence, or escaping
a specifically claimed control is.

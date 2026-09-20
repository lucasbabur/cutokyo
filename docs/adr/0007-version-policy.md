# ADR 0007: Independent version surfaces

- Status: Accepted
- Date: 2026-09-19

## Context

Application releases, persisted data, projections, capture envelopes, plugin peers,
and agent-facing commands evolve at different costs. One semantic version cannot
express safe downgrade and rebuild behavior for every surface.

## Decision

- Application versions come from release-tag semantic versions and remain 0.x until
  contracts stabilize.
- Database schemas use forward-only integers. New binaries migrate old schemas; old
  binaries clearly refuse newer schemas.
- Derived projections use an integer and rebuild when it changes.
- Spool formats use integers, and readers for every released format remain forever.
- Plugin protocols use an integer major handshake and reject unknown majors.
- Structured CLI and Cutokyo MCP surfaces follow application semantic versioning with
  one-minor deprecation because this explicit public contract overrides the default
  clean-break policy.

Private implementation APIs receive no compatibility shims. Unreleased fixtures do
not create a compatibility promise.

## Consequences

Upgrade tests need fixtures for every released database and spool version. Downgrade
cannot be best-effort. Removing a historical spool parser or silently accepting a
future major is a release blocker.

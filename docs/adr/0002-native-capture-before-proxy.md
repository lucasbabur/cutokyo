# ADR 0002: Native capture precedes consented proxy fallback

- Status: Accepted
- Date: 2026-09-19

## Context

Hooks, documented local APIs, OpenTelemetry, CLI output, and state files can establish
many facts without intercepting provider traffic. A proxy provides additional request
fidelity but raises a qualitatively different trust and credential boundary.

## Decision

The fixed source order is hook/plugin, local API, OpenTelemetry, harness CLI, local
state, file-watch trigger, provider usage API, consented proxy, labelled TUI scrape,
and labelled user declaration. Resolution picks the first channel that can establish
the fact without double counting lower tiers.

Automatic native fallthrough stops before proxy. Proxy use requires a durable explicit
consent record, loopback binding, a persistent visible active status, and disclosure
of captured content. It never persists credentials. Instrumentation fails open so the
coding harness continues. Provider requests pass unchanged; baseline redaction is
limited to retained metadata and separately consented analysis.

## Consequences

A native outage cannot silently turn Cutokyo into a MITM. Some facts, especially
context breakdown, remain unavailable until the user consents. UI and diagnostics
must distinguish capture, local diagnostics redaction, and consented analysis
rather than compressing them into a vague protected state. The outgoing guardrails
feature was removed by the product-scope change recorded in ADR 0010.

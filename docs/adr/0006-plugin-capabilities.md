# ADR 0006: Versioned subprocess plugins with explicit capabilities

- Status: Accepted
- Date: 2026-09-19

## Context

Plugins need extension points without becoming in-process database writers. Transcript
and network access are sensitive, while portable filesystem and network sandboxing is
not consistently available across supported desktop platforms.

## Decision

Plugins are external JSON-line subprocesses. They perform an integer-major handshake;
unknown majors are rejected before work. The host validates every inbound and outbound
message and applies explicit line, message, field, telemetry, output, queue, timeout,
and cancellation bounds. Errors name the message, field, expected value, and safe
actual description without exposing secrets.

Source plugins receive harness identity, approved capabilities, and cursor/window,
then emit immutable cursor-idempotent observations. Processor plugins receive
normalized records and optional separately approved transcripts, then emit tags,
derived facts, or nothing. Transcript and network capabilities require approval.
No plugin receives a database path or store handle.

Cutokyo describes these controls as capability grants and process isolation. It does
not claim filesystem or network sandboxing on platforms where none is enforced.

## Consequences

Plugins can be authored in any language and tested against JSON Schema. Subprocess
startup and validation have overhead. The verifier must run real good examples,
mutation fixtures, malformed-output recovery, cancellation, and timeout cases.

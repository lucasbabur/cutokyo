# Plugin authoring contract

## Contents

- [Status](#status)
- [Transport and handshake](#transport-and-handshake)
- [Plugin kinds](#plugin-kinds)
- [Capabilities](#capabilities)
- [Bounds and lifecycle](#bounds-and-lifecycle)
- [Validation and errors](#validation-and-errors)
- [Security boundary](#security-boundary)

## Status

The v1 schema and golden fixtures are foundation contracts. The subprocess host,
verifier, and executable examples are not yet implemented and are not claimed by
this document. Their implementation must conform to this contract rather than invent
an alternate wire format.

## Transport and handshake

A plugin is an external subprocess that exchanges one UTF-8 JSON value per line over
standard input and output. Every line validates against
[`schemas/plugin-protocol.v1.json`](../schemas/plugin-protocol.v1.json). Both host and
plugin send the same envelope fields: `protocol_major`, `kind`, `request_id`, and
`payload`.

The first message is a `handshake` with `protocol_major: 1`, a stable plugin ID,
`source` or `processor` kind, and requested capabilities. An unknown major is rejected
before work with the offending message, field, expected major, and safe actual major.
There is no best-effort fallback to another major.

## Plugin kinds

A source plugin receives harness identity, an approved capability set, and a
cursor/window. It emits immutable raw observations and must produce the same
observation IDs when the same cursor is retried.

A processor plugin receives normalized records and, only with separate approval,
transcript content. It emits tags, attributable derived facts, or no result. Every
derived fact links its source observation IDs.

## Capabilities

The v1 capability names are:

- `emit_observation`, valid for source output;
- `emit_derived_fact`, valid for processor output;
- `transcript_read`, requiring an explicit user grant;
- `network`, requiring an explicit user grant and visible destination policy.

Requesting a capability is not approval. The host passes only granted data and rejects
undeclared use. Plugins never receive a database path, connection, store handle,
keychain handle, or unrestricted app object.

## Bounds and lifecycle

The host implementation must enforce these v1 ceilings in both directions:

| Boundary | v1 ceiling |
| --- | ---: |
| UTF-8 JSON line | 1 MiB including newline |
| request ID | 128 bytes |
| plugin ID | 256 bytes |
| payload object | 64 top-level fields |
| capabilities | 8 unique values |
| telemetry per message | 64 KiB |
| emitted observations per response | 1,000 |
| emitted derived facts per response | 1,000 |
| concurrent requests per process | 32 |
| ordinary request timeout | 30 seconds |
| cancellation grace before termination | 2 seconds |
| process output after cancellation | 1 MiB |

The verifier must mutate each boundary and prove rejection. Timeouts and cancellation
are protocol outcomes. A malformed, oversized, timed-out, or cancelled run is cleaned
up so the next plugin invocation starts independently.

## Validation and errors

The host validates every outbound request before writing and every inbound response
before dispatch. Parsing into generic JSON and casting to a runtime type is not
validation. Unknown fields are rejected where the schema is exact.

Diagnostics identify direction, line/message, `request_id` when trustworthy, field,
expected contract, and sanitized actual description. Raw transcript text, secrets,
provider headers, and unbounded plugin output are never embedded in errors or logs.

Golden fixtures live under `fixtures/plugin/v1`. Good fixtures must validate; every
bad fixture must fail for its named reason. `cutokyo plugin verify` will execute the
real plugin and protocol host rather than accepting a manifest-only declaration.

## Security boundary

A subprocess boundary and capability grants reduce accidental privilege. They are not
a portable hard filesystem or network sandbox. Cutokyo must not claim such isolation
unless a platform-specific mechanism is actually enabled and reported. Users should
review plugin code and grants as they would any local executable.

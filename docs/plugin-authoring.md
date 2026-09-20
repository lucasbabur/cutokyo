# Plugin authoring contract

## Contents

- [Status](#status)
- [Manifest](#manifest)
- [Transport and handshake](#transport-and-handshake)
- [Plugin kinds](#plugin-kinds)
- [Capabilities](#capabilities)
- [Bounds and lifecycle](#bounds-and-lifecycle)
- [Validation and errors](#validation-and-errors)
- [Security boundary](#security-boundary)

## Status

The v1 schema, golden fixtures, bounded subprocess host, runtime verifier, and Python
source and TypeScript processor examples are implemented. `cutokyo plugin verify`
executes the declared plugin, validates the handshake and both message directions,
exercises source cursor idempotency where applicable, and waits for a clean bounded
shutdown. A successful receipt reports the exact protocol major, capabilities,
validated message count, idempotency check, and isolation boundary it proved.

## Manifest

Place an exact `cutokyo-plugin.json` at the plugin root. For example:

```json
{
  "manifest_version": 1,
  "protocol_major": 1,
  "plugin_id": "example:py-source",
  "plugin_kind": "source",
  "runtime": "python3",
  "entrypoint": "source.py",
  "args": [],
  "capabilities": ["emit_observation"],
  "timeout_ms": 5000
}
```

Unknown fields are rejected. The manifest is limited to 64 KiB. `runtime` is the
fixed allowlist `python3` or `node`; an arbitrary executable is not accepted. The
entrypoint must be a non-symlink regular file that canonicalizes beneath the plugin
root. A source must declare `emit_observation`; a processor must declare
`emit_derived_fact`. Up to 16 bounded arguments may follow the entrypoint, and the
request timeout must be between 1 and 30,000 milliseconds.

Run the real verifier from the repository or an installed binary:

```bash
cutokyo plugin verify ./my-plugin --json
```

The ordinary CLI verifier grants no sensitive capability. A host embedding the core
API may separately grant `transcript_read` or `network`; declaration alone is never
approval.

## Transport and handshake

A plugin is an external subprocess that exchanges one UTF-8 JSON value per line over
standard input and output. Every line validates at runtime against
[`schemas/plugin-protocol.v1.json`](../schemas/plugin-protocol.v1.json). Both host and
plugin use the envelope fields `protocol_major`, `kind`, `request_id`, and `payload`.

The first plugin message is a `handshake` with `protocol_major: 1`, a stable plugin
ID, `source` or `processor` kind, requested capabilities, and source idempotency when
applicable. An unknown major is rejected before work with the message direction,
index, field, expected major, and safe actual description. There is no best-effort
fallback to another major.

## Plugin kinds

A source plugin receives harness identity, an approved capability set, and a bounded
cursor/window. It emits immutable raw observations and must produce identical
observation results when the same cursor and window are retried.

A processor plugin receives normalized records and, only with separate approval,
transcript content. It emits attributable derived facts or no result. Every derived
fact links its source observation IDs. Neither kind may cross the other kind's output
boundary.

## Capabilities

The v1 capability names are:

- `emit_observation`, required for source output;
- `emit_derived_fact`, required for processor output;
- `transcript_read`, requiring an explicit host/user grant;
- `network`, requiring an explicit host/user grant and visible destination policy.

Requesting a capability is not approval. The host passes only approved data and
rejects missing declarations, unapproved sensitive capabilities, and undeclared use.
Plugins never receive a database path, connection, store handle, keychain handle, or
unrestricted application object.

## Bounds and lifecycle

The verifier enforces these v1 ceilings:

| Boundary | v1 ceiling |
| --- | ---: |
| UTF-8 JSON line | 1 MiB including newline |
| request ID | 128 bytes |
| plugin ID | 256 bytes |
| fields in every JSON object | 64 |
| manifest arguments | 16 |
| capabilities | 8 unique values |
| telemetry payload | 64 KiB |
| emitted observations per response | 1,000 |
| emitted derived facts per response | 1,000 |
| validated messages per verifier run | 256 |
| concurrent requests per process | 1 |
| ordinary request timeout | 30 seconds |
| cancellation grace before termination | 2 seconds |
| cumulative output after cancellation | 1 MiB |
| retained stderr | 64 KiB, never surfaced raw |
| graceful stdout closure after completion | 2 seconds |

Timeout covers the whole request rather than resetting on each telemetry message.
The host sends cancellation, bounds all later output, terminates the child, and drains
bounded stderr. Malformed, oversized, timed-out, cancelled, non-idempotent, or noisy
runs are cleaned up so the next invocation starts independently.

## Validation and errors

The host validates every outbound request before writing and every inbound handshake,
telemetry message, and response before dispatch. Deserialization alone is not treated
as schema validation. Unknown fields are rejected where the contract is exact.

Diagnostics identify direction, one-based message index, `request_id` only when its
syntax is trustworthy, field, expected contract, and sanitized actual description.
Raw transcript text, secrets, provider headers, stderr, and unbounded plugin output
are never embedded in errors or logs.

Golden fixtures live under `fixtures/plugin/v1`. Good fixtures validate; every bad
fixture fails for its named reason. The mutation suite covers unknown majors,
capability declarations and grants, malformed and oversized output, object/message
and item bounds, timeout/cancellation, source idempotency, forbidden output, and
recovery in a following valid run. Both shipped examples pass the same verifier used
by the CLI.

## Security boundary

A subprocess boundary, fixed runtime allowlist, exact entrypoint containment, and
capability grants reduce accidental privilege. They are not a portable filesystem or
network sandbox. Cutokyo does not claim such isolation unless a separately reported
platform mechanism actually enforces it. Users should review plugin code and grants
as they would any local executable.

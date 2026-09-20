# Plugin authoring contract

## Contents

- [Status](#status)
- [Manifest and static verification](#manifest-and-static-verification)
- [Transport and handshake](#transport-and-handshake)
- [Plugin kinds](#plugin-kinds)
- [Capabilities](#capabilities)
- [Bounds and lifecycle](#bounds-and-lifecycle)
- [Validation and errors](#validation-and-errors)
- [Security boundary](#security-boundary)

## Status

The v1 schema, golden fixtures, bounded manifest parser, protocol-major check, and
contained-entrypoint validation are implemented. `cutokyo plugin verify` currently
reports `runtime_validation: false`: it does **not** yet spawn the plugin or prove the
bidirectional handshake, timeout, cancellation, malformed-output recovery, or every
message bound. That subprocess conformance suite is a release blocker, not an implied
sandbox or a success hidden behind manifest validation.

A manifest-only success proves only the scope named in the JSON receipt. Authors can
use the wire contract below while the real host is completed; they must not describe
their plugin as runtime-verified yet.

## Manifest and static verification

Place an exact `cutokyo-plugin.json` at the plugin root:

```json
{
  "protocol_major": 1,
  "id": "example.safe-source",
  "name": "Safe source example",
  "kind": "source",
  "entrypoint": "plugin.py",
  "capabilities": ["emit_observation"]
}
```

Unknown fields are rejected. IDs are bounded to 128 bytes, names to 256 bytes, and
the manifest to 64 KiB. `kind` is `source` or `processor`. Capabilities use the exact
wire names `emit_observation`, `emit_derived_fact`, `transcript_read`, and `network`;
the latter two still require separate user grants.

The entrypoint must canonicalize to a regular file beneath the plugin root. A symlink
or `../` escape does not become trusted because it appears in a manifest. Run:

```bash
cutokyo plugin verify ./my-plugin --json
```

Inspect `verification_scope` and `runtime_validation` rather than treating `valid` as
a broader claim. Plugin manifests are configuration, not an operating-system sandbox.

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

| Boundary                              |              v1 ceiling |
| ------------------------------------- | ----------------------: |
| UTF-8 JSON line                       | 1 MiB including newline |
| request ID                            |               128 bytes |
| plugin ID                             |               256 bytes |
| payload object                        |     64 top-level fields |
| capabilities                          |         8 unique values |
| telemetry per message                 |                  64 KiB |
| emitted observations per response     |                   1,000 |
| emitted derived facts per response    |                   1,000 |
| concurrent requests per process       |                      32 |
| ordinary request timeout              |              30 seconds |
| cancellation grace before termination |               2 seconds |
| process output after cancellation     |                   1 MiB |

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
bad fixture must fail for its named reason. The release-complete verifier must execute
the real plugin and protocol host rather than accepting a manifest-only declaration;
the current JSON receipt explicitly identifies that missing runtime scope.

## Security boundary

A subprocess boundary and capability grants reduce accidental privilege. They are not
a portable hard filesystem or network sandbox. Cutokyo must not claim such isolation
unless a platform-specific mechanism is actually enabled and reported. Users should
review plugin code and grants as they would any local executable.

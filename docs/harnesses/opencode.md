# OpenCode native integration

## Evidence index

1. [Version and source matrix](#version-and-source-matrix)
2. [Installed read-only probe](#installed-read-only-probe)
3. [Capability and coverage matrix](#capability-and-coverage-matrix)
4. [Plugin lifecycle and event generations](#plugin-lifecycle-and-event-generations)
5. [Server reconciliation and finalization](#server-reconciliation-and-finalization)
6. [Setup and ownership](#setup-and-ownership)
7. [Inventory, accounts, and exact resume](#inventory-accounts-and-exact-resume)
8. [Fixture provenance and limits](#fixture-provenance-and-limits)

The adapter is native-first. The plugin is a doorbell and the local app server is the
ledger: the doorbell says something changed, while the exact-session ledger is reread
until the final record is stable. Neither channel enables a proxy. Proxy support is only
capability metadata for a separate extension layer with durable user consent.

## Version and source matrix

| Surface | Version/evidence | Official source | Established facts | Limits |
| --- | --- | --- | --- | --- |
| Installed executable | `opencode 1.18.28`, isolated read-only probe on 2026-09-20; executable path replaced by `<PRIVATE_EXECUTABLE_PATH_SENTINEL>` | `opencode --version`, `opencode --help`, `opencode session --help`, `opencode export --help` | Exact resume accepts `--session`/`-s`; `serve` supports loopback and port `0` | CLI version never chooses a wire parser; an unobserved version is labelled `unknown_version` |
| V1 plugin | Installed 1.18.28 plus exact tag `v1.18.28` (`22006d97652839999596a34a48ff6be7dbb40c6e`) | <https://opencode.ai/docs/plugins/>, <https://github.com/anomalyco/opencode/blob/v1.18.28/packages/plugin/src/index.ts> | Named plugin function returns an `event` callback; global/project plugin directories auto-load | Upstream may not await async shutdown work, so each Cutokyo spool publication is synchronous |
| Current V2 plugin | Current official docs observed 2026-09-20 | <https://opencode.ai/v2/docs/build/plugins>, <https://opencode.ai/v2/docs/build/plugins/migrate-v1> | `Plugin.define({id, setup(ctx)})`; `ctx.event.subscribe()` is live-only; `setup` returns cleanup | V2 setup is selected explicitly, never inferred from a payload or silently installed over V1 |
| Plugin npm artifact | `@opencode-ai/plugin@1.18.28`, SHA-256 `37efb341b471660fec94f7455d5e7bdfc24f8b835b1c16c648cc7f3dbf669878`, MIT | npm artifact and exact source tag | Root package declares legacy `{type, properties}` events | Installed 1.18.28 also emitted ID-bearing legacy events; an ID alone does not mean V2 |
| SDK npm artifact | `@opencode-ai/sdk@1.18.28`, SHA-256 `2592fb2b2861271290033320c1b975a347f3ee2cc3fc44fb777a066f23741745`, MIT | <https://opencode.ai/v2/docs/build/sdk> and npm artifact declarations | V2 session/message pages are `{data, cursor:{previous?,next?}}`; durable history is `{data,hasMore}`; events are `{id,type,data,...}` | Installed 1.18.28 serialized absent cursor members both by omission and explicit `null`; any other cursor value remains unknown |
| Local server | Official server docs plus isolated 1.18.28 observation | <https://dev.opencode.ai/docs/server/> | Root session/message/provider endpoints, `/event` SSE, `/api` V2 routes, and OpenAPI `/doc` | Only syntactic `localhost`, `127.0.0.0/8`, or `::1` HTTP origins are accepted; no port scan or remote host |
| CLI resume | Official CLI docs plus installed help | <https://dev.opencode.ai/docs/cli/> | Exact invocation is `opencode --session <recorded-id>` | `--continue` is forbidden because it means “latest,” not the recorded session |
| V1/V2 config, skills, MCP | Official configuration and discovery docs | <https://dev.opencode.ai/docs/config/>, <https://opencode.ai/v2/docs/config>, <https://opencode.ai/v2/docs/plugins>, <https://opencode.ai/v2/docs/skills>, <https://opencode.ai/v2/docs/mcp-servers> | V1 `plugin` and direct `mcp`; V2 `plugins`, `mcp.servers`, flat Markdown skills, and recursive `SKILL.md` | Presence/configuration does not prove runtime loading |
| Lifecycle defect evidence | Public issue plus deterministic fixture | <https://github.com/anomalyco/opencode/issues/16879> | Plugin async work can be lost at shutdown | Cutokyo publishes each event synchronously and still reconciles server history |

## Installed read-only probe

The locally installed `1.18.28` executable was launched with disposable `HOME` and XDG
roots, loopback-only serving, `--pure`, and no inherited provider credential variables.
No operator configuration, session, account, or credential store was read or changed.
The server process was stopped after the bounded probe.

| Request/command | Result | Safe conclusion |
| --- | --- | --- |
| `opencode --version` | `1.18.28` | executable and exact observed version only |
| `opencode --help` | `-s, --session session id to continue`; `--continue` says last session | exact-resume argument exists; “latest” is not equivalent |
| `GET /session` | `200`, JSON array | legacy session route is present in this isolated run |
| `GET /api/health` | `200`, object with boolean `healthy` | V2 health route is present |
| repeated `GET /api/session?limit=2` probes | `200`; one exact body used `{ "data": [], "cursor": {} }`, the final repeat used `{ "data": [], "cursor": {"previous":null,"next":null} }` | V2 cursor envelope and same-version absent/null drift are locally observed; emptiness proves only the disposable homes had no sessions |
| missing synthetic `/api/session/<id>/history` and `/message` | `404`, structured JSON | routes exist; no real session content was requested |
| `GET /event` | `200 text/event-stream`; first parsed field was `data` | legacy global live stream is present; replay is not proved |
| `GET /doc` | `200` OpenAPI document containing root and `/api` routes | route inventory for this process only |

The probe did not call an auth endpoint, read headers/tokens, inspect provider options, or
claim connected accounts. It did not prove V2 plugin runtime loading. Those remain
independent evidence dimensions.

## Capability and coverage matrix

| Capability | Strongest channel | Coverage | What is proved |
| --- | --- | --- | --- |
| Session events | V1 returned hook or V2 subscription, then server SSE | Partial for live-only plugin/global SSE; complete only for known replayable V2 session SSE | Immutable raw event, native ID/sequence when valid, and explicit disconnect reconciliation |
| Session/message history | Local app server | Complete for one coherent bounded V1 list or all exact V2 cursor pages | Exact raw pages and known items; endpoint generation drift restarts at page one |
| Durable event history | V2 local app server | Complete for known bounded `{data,hasMore}` pages | Pagination advances by strictly increasing native durable sequence |
| Final turn | Plugin wake-up plus exact-session local API | Complete only after stabilization | Known idle, terminal assistant message, and two identical canonical message-record snapshots |
| Exact resume | Native CLI | Complete when exact target exists | `opencode --session <exact-native-id>` with no reconstruction or nearby fallback |
| Usage | Local server | Partial | Tokens/cost in native records; no provider quota authority |
| Plugins, hooks, skills, MCPs | Config/state plus runtime evidence | Partial | Installed, configured, and loaded are separate evidence states |
| Connected accounts | Local server `/provider` | Partial | Only bounded IDs in `connected`; no auth or credential material |
| Context-component breakdown | None | Unavailable | Unknown, never zero |
| Proxy fallback | Separate extension layer | Consent required | Adapter provides capability metadata and normalization only |

File watching is never a truth source. It may trigger a bounded reread, whose provenance
remains `local_state` or `local_api`.

## Plugin lifecycle and event generations

Setup can generate either documented lifecycle, selected explicitly:

- V1 exports `Cutokyo`, whose returned `event` hook synchronously calls capture.
- V2 imports `Plugin` from `@opencode/plugin`, uses `Plugin.define`, consumes the live
  `ctx.event.subscribe({signal})` async iterable, and returns `() => controller.abort()`.

The subscription loop is asynchronous because it waits for events; publication of each
received event is not. The callback creates a unique file with create-new semantics,
writes one complete spool-v1 JSON line, `fsync`s, closes, atomically renames, and only
then handles the next event. Capture failure degrades open and emits a bounded status
when possible. V2 live subscriptions have no replay promise, so reconnect always leads
to server reconciliation.

`plugin_api` records which lifecycle delivered the callback; `event_api` records the wire
shape. They are not conflated. Known shapes are:

- V1: `{type, properties}`.
- Observed legacy hybrid: `{id, type, properties}` (`v1_with_id`).
- V2: `{id, type, data, durable?, location?, metadata?}`.
- V2 sync compatibility: `{type:"sync", id, syncEvent:{type,data,seq,...}}`.

An explicit unfamiliar API marker or incompatible shape remains untouched in raw
evidence with `unknown_version` coverage and produces no projection. A valid native
event ID supplies the deduplication identity; otherwise recursively key-sorted JSON is
SHA-256 fingerprinted. The original payload is never rewritten.

## Server reconciliation and finalization

V2 session/message pagination accepts only `{data, cursor:{previous?,next?}}`; a
`previous`/`next` member is either a bounded string or explicit `null` meaning absent,
as observed from installed 1.18.28. Invented `items`/`nextCursor` aliases are deliberately
unknown. V1 fallback accepts only a bare array. Durable history uses
`/api/session/{id}/history?limit=100&after=<number>`, while
per-session SSE replay uses the independent opaque `after` event cursor.

Every page is retained raw before flattening. A cursor loop, non-portable cursor, more
than 100 pages, more than 32 MiB, or non-increasing durable sequence fails visibly. If
OpenCode restarts or changes its port during pagination, pages from the old generation
are discarded and the operation restarts from page one on the newest plugin-observed
loopback origin. At most eight endpoint-generation restarts are attempted.

`session.idle` is a reconciliation trigger, not permanent completion. Duplicate idle
notifications do not reset progress. Finalization requires a terminal assistant record
(legacy `role:"assistant"` or V2 `type:"assistant"` with `time.completed`) and two
identical canonical message-record snapshots. Cursor metadata is excluded from that
stability digest. Pending candidates serialize as versioned
`OpenCodeFinalizationState`, so a Cutokyo restart or temporary server/port gap preserves
intent.

## Setup and ownership

Setup writes only the dedicated auto-loaded `plugins/cutokyo.ts`; it never rewrites
strict JSON or JSONC. The caller selects `OpenCodePluginApi::V1` for observed 1.18.28 or
`V2` only after establishing the V2 lifecycle. No version guess silently switches APIs.
A deliberate V1/V2 change records both the previously managed and intended lifecycle
before replacement. Recovery recognizes only those exact digests: before publication it
rolls state back to the still-present prior plugin; after publication it completes the
new lifecycle. A missing or third-party-edited file remains an explicit recovery error.

Before the first host mutation, versioned recovery intent is atomically persisted under
Cutokyo state. Planning records plugin/state snapshots; execution rechecks both before
and after acquiring the advisory setup lock, and rechecks the plugin immediately before
publication. The TypeScript file has a format-valid ownership marker. Setup refuses an
unmanaged collision, symlink, or non-regular target. New state/plugin files are
owner-only; an existing exact managed target's mode is retained.

Uninstall removes only a digest-matching managed plugin and leaves a restored tombstone
for repeat-cleanup idempotency. Because no unmanaged OpenCode file is ever modified,
there is no host config backup to restore; byte-for-byte avoidance is stronger. E2E
round trips verify an unrelated `opencode.jsonc` and its Unix mode remain unchanged.

Explicit cases cover dry-run, apply, repeat apply, interruption before/after plugin
publication, cleanup intent, partial plugin/state cleanup, recovery, prior restored
state, missing never-activated state, empty/corrupt state, concurrent edits, and repeat
uninstall. Missing managed state is a cleanup no-op. Empty or corrupt state is preserved
and reported rather than guessed.

## Inventory, accounts, and exact resume

Inventory structurally reads JSON/JSONC and documented roots. It understands V1
`plugin`, V2 `plugins` string/object entries, V1 direct `mcp`, V2 `mcp.servers`, direct
`.ts`/`.js` plugins, flat `.md` skills, and recursively nested `SKILL.md` up to bounded
depth/entry limits. It projects only sanitized names, scope-relative origins,
enablement, and evidence states. Raw option values are never retained.

A file or config entry proves only installed/configured. `loaded: yes` requires matching
runtime plugin evidence; skills remain load-unknown because loading is on demand.
Account hints come only from safe IDs in `/provider.connected`. The adapter never calls
auth, reads API keys, provider options, environment values, or credential stores.

Resume is available only when raw evidence established an exact portable native ID. The
argument vector is exactly:

```text
opencode --session <exact-recorded-native-session-id>
```

The fake process recorder proves the ID crosses the process boundary unchanged. Missing
or malformed targets return an actionable capability-unavailable error. There is no
`--continue`, latest-session lookup, path-derived ID, or reconstructed fallback.

## Fixture provenance and limits

`fixtures/harness/opencode/v1.18.28/manifest.json` is authoritative:

- `observed-sanitized/server-connected.json` and `session-created.json` came from the
  isolated installed 1.18.28 SSE/server exercise; IDs and private paths use named
  synthetic sentinels.
- `observed-sanitized/v2-empty-session-page.json` and
  `v2-empty-session-page-null-cursors.json` are the two exact empty bodies returned by
  isolated installed `GET /api/session?limit=2` probes. They preserve omission-versus-null
  cursor drift and contain no private values.
- `documented-synthetic/v1-session-idle.json`, `v2-paginated-history.json`, and
  `finalization-snapshots.json` are generated from exact documented/package-declared
  shapes plus explicit synthetic IDs. They are not claimed as observed traffic.
- `unknown-plugin-generation.json` is an adversarial future-drift fixture.

Core tests feed the versioned page fixtures into shape negotiation; fake-harness tests
exercise pagination, durable events, lifecycle lag, unknown drift, exact resume, and a
port change during page one that proves stale pages are discarded.

Bounds are 8 MiB per plugin event/spool line, 32 MiB per server operation, 100 history
pages, 10,000 SSE events, 1 MiB per SSE event, eight endpoint restarts, and 500 MiB
retained plugin spool bytes. Capacity exhaustion is visible and never silently deletes
older evidence.

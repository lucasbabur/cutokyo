# Reference observation ledger

These are behavior-only observations supplied by the authoritative Fleet context.
They are not permission to consult the predecessor, and no expression, schema,
fixture, UI, asset, test, or implementation was copied. All requirements below were
independently restated and are corroborated by public documentation where possible.

| Key | Independently stated reference observation | Why retained | Public corroboration |
| --- | --- | --- | --- |
| RO-001 | Local services should bind to loopback and distinguish approved provider destinations. | Prevent accidental remote exposure and confused egress. | Tauri capability guidance; standard loopback and URL validation practices. |
| RO-002 | Persist recovery intent before changing another tool's configuration; roll back independently owned subsystems independently. | Crashes and uninstall must not destroy neighboring user config. | Atomic-file patterns and provider configuration documentation. |
| RO-003 | Provider-bound traffic and persisted or telemetry payloads are different protection boundaries. | Redacting logs must not be advertised as changing model requests. | Provider API and OpenTelemetry documentation. |
| RO-004 | Subsystem degradation must be durable and visible after restart. | In-memory green status can conceal failed persistence or quarantine. | SQLite durability and health-check practice. |
| RO-005 | Quarantine malformed input durably before acknowledging or advancing a cursor. | A crash must not lose the only copy of rejected evidence. | Transactional queue and SQLite durability principles. |
| RO-006 | Files, lines, payloads, queues, plugin output, and diagnostics need explicit bounds. | External producers can otherwise exhaust memory or disk. | MCP/JSON-RPC transport guidance and secure parser practice. |
| RO-007 | Session projection needs stable attributable identity and idempotent ingest. | Duplicate hooks and identity drift must not double count or resume the wrong target. | Claude Code hooks, Codex app-server, and OpenCode plugin documentation. |
| RO-008 | Overlapping reads and stale responses must not overwrite newer session/project state. | UI truth should follow request ordering rather than network timing. | General cancellation and stale-response handling practice. |
| RO-009 | Process liveness and product readiness are distinct. | A running shell can still have an unavailable database, spool, or plugin plane. | Health endpoint conventions and Tauri startup behavior. |
| RO-010 | Package smoke tests must launch the package, not a source-tree binary. | Source success does not prove installer or wrapper correctness. | cargo-dist npm and Tauri distribution documentation. |
| RO-011 | Health failures must remain independent and current versus lifetime quarantine must be distinct. | One successful write cannot erase unrelated backup, integrity, or retention faults. | Newly authored bounded health contract. |
| RO-012 | Uninstall with no activation state is a successful no-op; partial recovery remains diagnosable. | Fresh or repeated uninstall should not become a product trap. | Idempotent cleanup practice and host config ownership rules. |
| RO-013 | A parallel process launch can temporarily inherit another thread's writable executable descriptor before `exec` applies close-on-exec, causing Linux to reject execution with `ETXTBSY`. Executable test fixtures must therefore be completely published before parallel launch and remain unopened for writing while tests run. | Prevent nondeterministic false Codex-unavailable failures without retries, sleeps, or test serialization. | POSIX `exec` documents `ETXTBSY`; Linux `open(2)` documents that `O_CLOEXEC` acts during successful `exec`; independently reproduced on candidate `7468bb4` and corroborated by public Rust and Go issue reproductions. |

## Public sources

- Tauri capabilities, distribution, updater, and native testing:
  <https://tauri.app/security/capabilities/>,
  <https://v2.tauri.app/distribute/>,
  <https://v2.tauri.app/plugin/updater/>,
  <https://v2.tauri.app/develop/tests/webdriver/>.
- SQLite WAL, transactions, backup, FTS5, and network-filesystem guidance:
  <https://www.sqlite.org/wal.html>, <https://www.sqlite.org/lang_transaction.html>,
  <https://www.sqlite.org/backup.html>, <https://www.sqlite.org/fts5.html>,
  <https://www.sqlite.org/useovernet.html>.
- Claude Code hooks and monitoring:
  <https://code.claude.com/docs/en/hooks>,
  <https://code.claude.com/docs/en/monitoring-usage>.
- Codex app server and CLI: <https://developers.openai.com/codex/app-server>,
  <https://developers.openai.com/codex/cli>.
- OpenCode V1/V2 plugins, server, CLI, config, skills, MCP, SDK, and exact 1.18.28 source:
  <https://opencode.ai/docs/plugins/>, <https://opencode.ai/v2/docs/build/plugins>,
  <https://opencode.ai/v2/docs/build/plugins/migrate-v1>,
  <https://dev.opencode.ai/docs/server/>, <https://dev.opencode.ai/docs/cli/>,
  <https://dev.opencode.ai/docs/config/>, <https://opencode.ai/v2/docs/config>,
  <https://opencode.ai/v2/docs/plugins>, <https://opencode.ai/v2/docs/skills>,
  <https://opencode.ai/v2/docs/mcp-servers>, <https://opencode.ai/v2/docs/build/sdk>,
  <https://github.com/anomalyco/opencode/blob/v1.18.28/packages/plugin/src/index.ts>,
  <https://github.com/anomalyco/opencode/blob/v1.18.28/packages/opencode/src/plugin/index.ts>.
- OpenCode plugin lifecycle non-awaited-promise report:
  <https://github.com/anomalyco/opencode/issues/16879>.
- Official Rust MCP SDK: <https://github.com/modelcontextprotocol/rust-sdk>.
- cargo-dist npm installer:
  <https://axodotdev.github.io/cargo-dist/book/installers/npm.html>.
- Process execution and Linux `ETXTBSY` semantics:
  <https://pubs.opengroup.org/onlinepubs/9799919799/functions/exec.html>,
  <https://www.man7.org/linux/man-pages/man2/open.2.html>,
  <https://github.com/rust-lang/rust/issues/114554>,
  <https://github.com/golang/go/issues/22315>,
  <https://lkml.rescloud.iu.edu/hypermail/linux/kernel/2508.3/03883.html>.

## Fixture statement

Foundation fixtures under `fixtures/` are synthetic and were authored from this
contract. Dates, identifiers, payloads, ordering, and expected failures are invented,
except the four files explicitly classified `locally_observed_sanitized` by
`fixtures/harness/opencode/v1.18.28/manifest.json`. Those JSON values came from
isolated, credential-free local OpenCode 1.18.28 loopback observations; every native ID
and private path was replaced by an explicit sentinel, while the two empty V2 pages
needed no redaction. The remaining OpenCode files
are labelled documented-synthetic or adversarial and are not represented as observed
traffic. No predecessor fixture or production transcript was copied.

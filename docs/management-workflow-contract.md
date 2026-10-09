# Desktop management command contract

## Session search

`searchSessions(filters)` requires text, queryMode, sort, offset, limit, todayStart,
harness, project, branch, dateRange, tool, skill, and agent. Empty text and `all`
filters search stored history. The default page is 50 sessions. A page is bounded
to 500, but its offset is not: older history remains reachable.

`queryMode: "terms"` requires every literal Unicode word across the selected
session title, project name/path, branch, native identity, or transcript.
Punctuation separates words; `OR`, `NOT`, quotes, and `*` are not FTS operators.
`queryMode: "phrase"` requires consecutive words in one metadata field or one
canonical message. It never invents a phrase across message boundaries.

`sort: "relevance"` uses native SQLite FTS5 BM25 with higher weights for metadata.
`sort: "newest"` uses session start time. Both resolve ties by start time and
session ID. Duplicate observation delivery does not add selected transcript text
or change relevance. With no text, results are newest first.

The response has exact `total`, effective `offset` and `limit`, `hasMore`, and
actual observed project, branch, tool, skill, and agent facets across all retained
history, not only the first page. Sessions include up to three `matches` with
`source`, plain `text` bounded to 320 Unicode characters, and literal `terms`.
The UI renders safe React text and highlights, never returned HTML. Full
transcripts load only on the detail route.

Today means machine-local calendar midnight, including daylight-saving changes.
The client recomputes RFC3339 `todayStart` for each request; native code rejects
missing, future, or stale boundaries. Other date ranges remain rolling day ranges.

History debounces text by 250ms. While loading or after failure, previous results
stay visible and explicitly labeled stale. Errors remain visible with Retry.
Search, filters, sorting, and page survive the exact detail/back-link journey in
validated session storage. Clearing a no-match query restores history; an empty
offset page offers a return to the first page. Collapsed More filters names every
active project, branch, tool, skill, and agent constraint, including retained
values that are no longer observed.

Overview totals intentionally cover at most the latest 500 sessions. Their route
metadata visibly reports the bounded count and exact retained-history total
whenever more sessions exist. Sessions search reaches all retained history.

CLI examples use `cutokyo sessions search --query 'database migration' --page`,
`--phrase`, `--sort newest`, and `--offset 500 --limit 50`. Without `--page`, the
published structured CLI array shape remains available under its versioned
contract. The read-only MCP `session_search` input adds `query_mode`, `sort`,
and `offset`; its response includes exact page metadata and plain match objects.
The MCP page limit remains 100.

Schema 3 and derive version 3 rebuild selected-session and canonical-message
FTS projections from stored evidence. Native global events and unknown OpenCode
event generations remain raw evidence without fabricated sessions, search rows,
or resume authority. Search regression worlds are independently
authored synthetic observations in `store_search_tests.rs`; no predecessor code,
fixtures, transcripts, or assets are used. Browser fixtures exercise the same
terms/phrase, filtering, paging, navigation, and plain-text rendering workflows;
their relevance scores and non-Latin tokenizer are approximations, not proof of
SQLite BM25 or unicode61 behavior. Native SQL tests are authoritative for those.

Implementation contract for the management-first change. The native core must enforce every capability, not trust the UI's disabled state. Requests refer to discovered item IDs, never arbitrary user-supplied file paths.

## Inventory document

`getInventoryDocument(itemId)` returns:

```ts
interface InventoryDocument {
  itemId: string;
  content: string;
  format: "markdown" | "json" | "toml" | "text";
  revision: string;
  editable: boolean;
  removable: boolean;
  unavailableReason: string | null;
  installTargets: readonly {
    harness: "claude_code" | "codex" | "opencode";
    available: boolean;
    reason: string | null;
    destination: string;
  }[];
}
```

A revision identifies the exact source snapshot shown to the user. Read-only items return an explicit explanation. A target is unavailable if the harness cannot represent the source item, it already exists, or the source scope cannot be mapped safely. Never silently overwrite a destination or claim generic hook/plugin portability.

`saveInventoryDocument(itemId, revision, content)` returns `ActionReceipt`.

`removeInventoryItem(itemId, revision)` returns `ActionReceipt`.

`installInventoryItem(itemId, revision, harness)` returns `ActionReceipt`.

The receipt is the existing `{ ok: boolean; message: string; status: "success" | "cancelled" | "unavailable" }` contract. Every mutation rechecks source revision, refuses unsafe filesystem targets, preserves unrelated configuration and permissions, and retains a recovery copy. Install rechecks destination absence. Skill installation includes required supporting files, not just SKILL.md. Changes operate through core application use cases; frontends do not write harness configuration.

The native Tauri command names are `inventory_document`, `save_inventory_document`, `remove_inventory_item`, and `install_inventory_item`. Arguments use `itemId`, `revision`, `content`, and `harness`.

## UI behavior

The inventory page has a name/content search, kind filters, harness filter, and refresh. Every row has a Manage action. Its dialog loads the document, displays actual source content and scope, offers Save changes only if editable, installation with exact target and reason, and removal only if removable. A separate confirmation names the exact item and source. Error messages live inside the active dialog. Failed saves preserve input. In-progress writes cannot be dismissed or submitted twice. Successful mutations close the dialog, announce their receipt, and reload the real inventory.

Protected Cutokyo-owned capture hooks link to Manage native capture rather than offering direct document edits or removal. Navigation uses native ownership and capability flags, not the wording of an unavailable explanation. Unmanaged hooks and other read-only tools do not receive that link.

## Native capture management

Settings permanently links to `#/onboarding`. Before onboarding completes, this route offers first-run selection and completion. After completion, it presents native capture management with the existing per-harness preview, install, verification, recovery, and removal controls. Session history does not hide the entrypoint.

Management still requires acknowledgement of local storage before configuration changes. Preview cancellation writes nothing. Pending actions cannot be submitted twice, and Return to Settings is disabled during an operation. Errors remain actionable in the page; failed operations clear stale local configuration-verification claims.

Visiting management or changing native configuration does not reset onboarding, selected harness preferences, session history, appearance, or unrelated settings. Saved harness selection is intent, not installation evidence. Removal changes the actual native files and shows unverified configuration while retaining that intent. It does not revoke callbacks already loaded by a running harness; restart that harness to unload them. Successful configuration verification never claims that live capture was observed.

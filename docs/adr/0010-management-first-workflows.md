# ADR 0010: Management-first desktop workflows

## Brief

Improve the existing Cutokyo desktop for people using Claude Code, Codex, and OpenCode. The main job is to manage installed agent tools and recover useful session history. Keep the existing light/dark appearance, local-first storage, and native application boundaries. Build directly into the existing design system. No decorative imagery is needed.

The user requested editable and removable skills, MCP servers, hooks, and plugins, installation into another harness, simpler backups, better session search, and removal of the guardrails feature.

## Research and reference lock

Refero style searches on 2026-10-04 returned `NO_RESULTS` for both detailed queries and broad `Linear`, `developer`, and `minimal` queries. Use the existing appearance system in ADR 0009 as the visual target, with Refero's bundled typography and color craft references. Do not claim newly retrieved style references.

Live Refero screen research:

- Linear connected accounts, `6faefb43-55c6-4c99-a36c-19b02bbea27c`. Single-column sections, compact integration rows, status and actions adjacent to their target.
- Lemon Squeezy integrations, `6d794fb3-74e2-4e20-ba28-51d51dfed4ca`. Separates item identity and description from right-aligned configuration actions. Inspected the screen image.
- Cofounder integrations, `038caadf-9b9c-4e52-99d4-5f7fb05901b1`, and disconnect flow `7904`. Searchable installed items, a specific removal confirmation, refreshed state, and success feedback.
- Monday search, `fa6d1a2e-e00a-4776-b696-1794f83b8431`. Search result context and explicit relevance/time sorting. Inspected the screen image.
- MyMind local export flow `13325`. Explain contents, select a local destination, show an in-progress state, and give a completion receipt. Borrow the sequence, not its typography or export limitations.

Primary direction: the existing Cutokyo compact workspace. Preserve its sidebar, typography scale, restrained neutral surfaces, hairline borders, sparse cyan focus/link color, and independent light/dark palettes. Borrow the integration row/action structure and search controls above. Do not introduce decorative gradients, marketing cards, extra typefaces, or status colors used as decoration.

## Decision ledger

| Decision | Source | Role | Reason |
| --- | --- | --- | --- |
| Keep existing theme tokens and typography | ADR 0009; bundled typography/color craft references | Structural surfaces and readable compact product UI | Preserve in-progress appearance work and avoid an unrelated redesign |
| Rename discovery-oriented language to management language | User request | Page title and action copy | People visit to change their installed tools, not inspect a technical report |
| Search and filter installed items by kind and harness | Cofounder integration screen; user request | Navigation within local inventory | A long list needs a fast route to a specific skill or server |
| Put edit, remove, and install actions beside supported items | Linear and Lemon Squeezy integrations | Item-specific controls | The next action should not require guessing another page |
| Show exact destination and unsupported reasons | User request; native adapter constraints | Mutation scope and capability truth | Do not promise portable hooks or overwrite existing configuration implicitly |
| Keep capture management reachable after onboarding | User's hook-management goal; Linear integration rows; native journey audit | Permanent Settings entrypoint and protected-hook link | Existing session history must not hide install, recovery, or removal controls |
| Confirm removal, retain editor input on error, refresh on success | Cofounder disconnect flow 7904 | Recovery and completion | Avoid accidental deletion and false success |
| Show matching session context and relevance/time sorting | Monday search screen | Result interpretation | Users need to see why a session matched |
| Keep search filters when opening a result and returning | User's session search goal | Navigation state | Searching again after every inspection wastes work |
| Make backup an explicit primary action with a sensible default location | MyMind export flow 13325; user request | Local data recovery | Creating a recovery copy should not require constructing CLI commands or digests |
| Keep restore verification in the application, not manual user input | Cutokyo backup contract; user request | Integrity and destructive restore | Simpler interaction must still create recoverable, validated data |
| Remove guardrails as a shipped product feature | User request | Feature scope | Defer secret blocking/detection rather than retaining a hidden dual path |

## Boundaries

All real mutations are core application use cases. Browser fixtures model the same requests and errors, but are not evidence of native support. Do not mutate the developer's installed harness configuration during tests. Use isolated configuration roots and genuine adapter files.

Removing the guardrails feature does not remove file integrity validation, symlink protection, backup digests, database integrity checks, diagnostics redaction, or provider-egress consent. These are ordinary correctness and privacy requirements, not a guardrails product.

## Verification

Run the gate mutation selftest before relying on its result. Add regression tests for real configuration mutations, stale edits, existing destinations, unsupported harness formats, backup/restore, session query ranking and previews, UI failures and cancellation. Exercise the integrated browser and native application and inspect screenshots at 900x700, 1280x800, and 1536x960 in both themes. Do not weaken an acceptance command to hide a failed check.

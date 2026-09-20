# Clean-room implementation policy

## Purpose

Cutokyo community is independently implemented under Apache-2.0. The local
predecessor at `/home/lucas/Developer/personal/cutokyo` has no repository-wide license
grant. Public visibility is not permission to copy expression.

## Permitted inputs

Contributors may use:

- this repository's authoritative, independently worded product contract;
- the behavior and failure-mode observations already recorded in Fleet context and
  restated in `reference-observations.md`;
- public provider, SQLite, Tauri, MCP, Rust, npm, and standards documentation;
- newly authored synthetic data and tests;
- dependencies and assets with verified compatible licenses.

## Prohibited inputs

Do not read, copy, translate, paraphrase line-by-line, execute, import, diff, or use as
a fixture source any predecessor code, schema, test, data, fixture, prose, UI/CSS,
image, icon, font, workflow, generated plan, agent scaffold, commit history, or private
endpoint assumption. No code or expression from that repository enters this history.
Do not migrate or inspect user data from an old installation.

If accidental exposure occurs, stop work, identify the affected files without copying
the material into an issue, and ask a maintainer for clean-room remediation. Rewriting
from memory is not an acceptable shortcut.

## Required record

Before product source or tests are committed:

1. `reference-observations.md` records each behavior-only input, its need, and public
   corroboration.
2. `decision-ledger.md` maps independently written requirements to new implementation
   and test locations and records review status.
3. `THIRD_PARTY.md` records dependency and asset licenses.

Each later contributor updates the decision ledger in the same commit as new behavior.
Fixtures identify whether they are synthetic or derived from public documentation.
Observed provider fixtures require origin, version, collection method, sanitization,
and a statement that no predecessor expression was copied.

## Review rule

Reviewers judge contract conformance without opening the predecessor. Similar product
ideas are not by themselves copying; identical expression, layout, fixture values,
or internal structure without an independent reason is a blocker. When provenance is
uncertain, omit the material until a clean source is established.

## What changes

Describe the user-visible behavior and why it belongs in the public local-only product.

## Boundaries and provenance

- [ ] I did not read or copy predecessor code, schemas, tests, fixtures, prose, UI,
      assets, or workflows.
- [ ] I updated `docs/provenance/decision-ledger.md` for new behavior/tests.
- [ ] I recorded every new dependency or asset and verified its license.
- [ ] I preserved raw evidence, provenance, unknown states, and app/store boundaries.

## Verification

List exact commands and exit codes. Run static verifier selftests before static gates.

- [ ] Rust formatting, Clippy, tests, architecture, and contracts pass.
- [ ] TypeScript type/lint/format/test checks pass where affected.
- [ ] Good and targeted bad fixtures cover contract changes.
- [ ] I did not skip/focus tests, weaken thresholds, or substitute easier fixtures.

## DCO

- [ ] Every commit has my matching `Signed-off-by` trailer (`git commit -s`).

# Contributing to Cutokyo

## Contents

- [Before changing code](#before-changing-code)
- [Development checks](#development-checks)
- [Developer Certificate of Origin](#developer-certificate-of-origin)
- [Pull requests](#pull-requests)

## Before changing code

Read the product specification, architecture decisions, and
`docs/provenance/clean-room-policy.md`. The predecessor named there is not a source
donor: do not read or copy its code, tests, fixtures, schemas, prose, UI, assets, or
workflows. Update the decision ledger with every new behavior and test.

Use English for product text, code, docs, schemas, fixtures, issues, and pull requests.
Prefer maintained, license-compatible dependencies after documenting release recency,
fitness, and license. Do not add generated or third-party assets without provenance.

## Development checks

Run verifier selftests before trusting static results:

```bash
python3 tools/fleet/cutokyo-gates.py selftest
python3 tools/fleet/cutokyo-gates.py all --root .
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check licenses bans
pnpm --dir ui check
```

Do not skip tests, focus tests in committed code, weaken bounds, replace adverse
fixtures with easy data, or suppress valid diagnostics. A broken check is changed only
with a demonstrated contradiction and a regression selftest.

## Developer Certificate of Origin

Cutokyo uses the Developer Certificate of Origin 1.1. Sign every commit with your real
name and email:

```bash
git commit -s
```

This adds a `Signed-off-by: Name <email>` trailer. The sign-off certifies the following:

> Developer Certificate of Origin
>
> By making a contribution to this project, I certify that:
>
> (a) The contribution was created in whole or in part by me and I have the right to
> submit it under the open source license indicated in the file; or
>
> (b) The contribution is based upon previous work that, to the best of my knowledge,
> is covered under an appropriate open source license and I have the right under that
> license to submit that work with modifications, whether created in whole or in part
> by me, under the same open source license (unless I am permitted to submit under a
> different license), as indicated in the file; or
>
> (c) The contribution was provided directly to me by some other person who certified
> (a), (b) or (c) and I have not modified it.
>
> (d) I understand and agree that this project and the contribution are public and
> that a record of the contribution (including all personal information I submit with
> it, including my sign-off) is maintained indefinitely and may be redistributed
> consistent with this project or the open source license(s) involved.

Automated or agent-assisted contributions need the human author's matching sign-off
and any attribution trailers required by the contribution workflow. CI enforces DCO
on every pull-request commit.

## Pull requests

Keep commits coherent and reviewable. Explain the user behavior, boundaries, tests,
provenance, and docs. Include exact commands and exit codes. Do not push generated
release artifacts, credentials, transcripts, personal paths, or production data.

By participating, you agree to the Code of Conduct. Report security issues through the
private process in `SECURITY.md`, not a public issue.

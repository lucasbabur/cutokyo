# Release signing, notarization, and provenance

## Contents

- [Trust boundaries](#trust-boundaries)
- [Authorized trigger](#authorized-trigger)
- [Required secrets](#required-secrets)
- [macOS signing and notarization](#macos-signing-and-notarization)
- [Windows Authenticode](#windows-authenticode)
- [Tauri updater signatures](#tauri-updater-signatures)
- [CLI, npm, checksums, and SBOM](#cli-npm-checksums-and-sbom)
- [Local production Linux package](#local-production-linux-package)
- [Verification and rotation](#verification-and-rotation)

## Trust boundaries

Four controls solve different problems and are never presented as substitutes:

1. **platform code signing** identifies the desktop publisher to macOS or
   Windows;
2. **Apple notarization** records Apple's automated review and allows ticket
   stapling;
3. **Tauri updater signatures** authenticate update payloads to an already
   installed app;
4. **SHA-256, CycloneDX, and build provenance** inventory and attest release
   bytes but do not create a platform identity.

Linux checksum/provenance success does not imply macOS notarization or Windows
Authenticode. An updater `.sig` does not imply platform code signing.

## Authorized trigger

`.github/workflows/release.yml` has two modes:

- manual `workflow_dispatch` is a non-publishing dry-run;
- a semantic-version tag is the only publishing path.

The tag path runs a signing-secret preflight before artifact jobs. If any
required credential is absent, the workflow exits 78 with the missing variable
names and publishes nothing. Pull-request tests and dry-runs never publish,
create a release, or silently fall back to unsigned output.

Review the commit, DCO trailers, CI result, generated cargo-dist plan, version,
and tag before authorization. Tag creation and secret access are release-manager
actions outside ordinary test automation.

## Required secrets

Tag builds require:

| Protected value                       | Storage               | Purpose                                  |
| ------------------------------------- | --------------------- | ---------------------------------------- |
| `NPM_TOKEN`                           | environment secret    | npm publication with provenance          |
| `TAURI_SIGNING_PRIVATE_KEY`           | environment secret    | updater payload signatures               |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`  | environment secret    | decrypt updater signing key              |
| `TAURI_UPDATER_PUBLIC_KEY`            | repository variable   | reviewed updater verification anchor     |
| `APPLE_CERTIFICATE`                   | environment secret    | base64 PKCS#12 Developer ID certificate  |
| `APPLE_CERTIFICATE_PASSWORD`          | environment secret    | import PKCS#12 into ephemeral keychain   |
| `APPLE_SIGNING_IDENTITY`              | environment secret    | select Developer ID Application identity |
| `APPLE_ID`                            | environment secret    | notarization account                     |
| `APPLE_PASSWORD`                      | environment secret    | app-specific notarization password       |
| `APPLE_TEAM_ID`                       | environment secret    | notarization team                        |
| `WINDOWS_CERTIFICATE`                 | environment secret    | base64 PKCS#12 Authenticode certificate  |
| `WINDOWS_CERTIFICATE_PASSWORD`        | environment secret    | import Windows certificate               |

Store credentials only in protected GitHub environments or repository secrets
with least-privilege release access. The public key is deliberately not a secret,
but its repository-variable value is still a reviewed release trust anchor: two
reviewers must compare it to the public half of the protected private key before
enabling a tag. Never place private keys, passwords, or certificates in TOML,
source, build logs, artifacts, diagnostic bundles, or workflow inputs.

## macOS signing and notarization

The macOS job decodes the certificate into the runner's temporary directory,
creates and unlocks an ephemeral keychain, imports the identity, and grants only
the signing tool partition access. Tauri receives the identity and Apple
notarization environment variables. The release build waits for notarization and
stapling; it does not use `--skip-stapling` in the ordinary release path.

After build, verify on a clean macOS machine:

```bash
codesign --verify --deep --strict --verbose=2 PATH_TO_APP
spctl --assess --type execute --verbose=4 PATH_TO_APP
xcrun stapler validate PATH_TO_APP
```

Remove temporary certificate bytes and keychains even after failure. Never print
`security find-identity` output into a public artifact if it contains identifying
material beyond the reviewed certificate name.

## Windows Authenticode

The Windows job decodes and imports the PKCS#12 certificate into the ephemeral
runner user's personal store. Its thumbprint enters an untracked, runner-temporary
Tauri configuration overlay together with SHA-256 and a trusted timestamp URL.
The checked-in config never contains private key material.

Verify the installed executable and installer on a clean Windows machine with
PowerShell `Get-AuthenticodeSignature`. Require `Valid`, the expected publisher,
and a timestamp. Remove the imported certificate and temporary PFX after the
build. A Tauri updater signature alone is not an acceptable Windows signature.

## Tauri updater signatures

The tag jobs run `tools/release/prepare-signing-config.py` to create a temporary
Tauri overlay. It enables updater artifacts and embeds the reviewed public key in
`plugins.updater.pubkey`. On Windows it also selects the imported Authenticode
certificate. It never reads or writes the private signing key. Ordinary builds do
not enable updater artifacts.

Tauri 2.11.4 signs `.AppImage` and `.msi` payloads directly. macOS uses `.app.tar.gz`.
The workflow does not request the deprecated v1-compatible archive format. Each
job declares its target triple; the macOS ARM runner produces `darwin-aarch64`,
not a mislabeled Intel update. Release assembly refuses a tag release unless
signed updater payloads exist for Linux, macOS, and Windows.

`tools/release/create-manifest.py` verifies every bounded `.sig` against its
payload, including signed `.deb` files not selected for the updater. It creates
`latest.json` from explicit architecture-to-payload mappings and immutable release
URLs. It
strictly decodes Tauri's outer Base64, parses the enclosed Minisign text, and uses
the checked-in Rust verifier to verify every payload byte against the reviewed
`TAURI_UPDATER_PUBLIC_KEY` repository variable. It rejects orphaned, malformed,
duplicate-platform, mismatched, or missing required signatures. Tagged assembly
fails before publication if the public anchor is absent or does not match the
protected private key. The private key remains exclusively in the release
environment; the workflow materializes the public anchor only in runner-temporary
storage and removes that runner with the job.

## CLI, npm, checksums, and SBOM

cargo-dist 0.32.0 builds target-native `cutokyo` archives and generates shell,
PowerShell, and npm installers. The npm project is packed locally and smoke
tested by installing its tarball, letting its real postinstall consume a local
cargo-dist native archive, resolving `node_modules/.bin/cutokyo`, and checking
structured version and doctor output. The installed executable must hash to the
same bytes as the executable inside that archive. Linux smoke uses a loopback-only
network namespace and a native-socket denial probe, not just proxy variables.
It requires bubblewrap or an existing network-disabled container. Other platforms
report their narrower Node-only network check without claiming native confinement.
The smoke uses an allowlisted child environment and disposable HOME, config,
data, and npm-cache directories.

Release assembly emits:

- cargo-dist per-artifact SHA-256 files;
- a complete `SHA256SUMS` inventory;
- a machine-readable release manifest;
- CycloneDX JSON SBOMs from pinned `cargo-cyclonedx`;
- GitHub build-provenance attestations;
- the cargo-dist plan used for the release.

Every build and package command is wrapped by the tracked-source snapshot guard.
Exit 86 means the command rewrote tracked source and the release must stop.

Publication is draft-first. After every artifact, smoke test, signature check,
checksum, manifest, SBOM, and provenance step succeeds, the workflow creates a
draft GitHub release from the reviewed bytes. It then publishes the already-packed
npm artifact with npm provenance. Only after npm succeeds does it make the GitHub
release public. A failed npm publication therefore leaves an inspectable draft
rather than advertising a release whose installer channel is absent; no workflow
step rebuilds bytes between those operations.

## Local production Linux package

When the host lacks WebKit development packages, use the reviewed local
`cutokyo-tauri-builder:2.11.4` image. Install the locked pnpm workspace first, then
run this from a clean committed checkout with `OUT` set to a new external path.
Set `PNPM_ROOT` to an unpacked official pnpm 11.25.0 distribution containing
`bin/pnpm.mjs`. The builder mounts only that dependency directory read-only and
bypasses Corepack's download-on-first-use behavior inside the offline container.
`CARGO_CACHE` must contain the fetched public `git/` and `registry/` dependencies.
Only those two subdirectories are mounted, read-only. Cargo configuration and
credential files are not mounted.

```bash
python3 tools/release/build-linux-package.py --root "$PWD" --output "$OUT" --pnpm-root "$PNPM_ROOT" --cargo-cache "$CARGO_CACHE"
```

The builder records the exact Git revision, tree, resolved Docker image ID,
recipe hash, source-immutability receipt, `.deb` hash, and contained executable
hash. It builds the release profile with `desktop-runtime`, without `native-e2e`,
and never launches the app. It mounts no harness state or host credentials.
Missing offline dependencies are an environment failure, not a passing build.

Run install/start/uninstall and GUI QA separately, with no parallel native runner.
`smoke-tauri-linux.py` installs the `.deb` with dpkg, resolves its owned `/usr/bin`
executable, compares it with the packaged executable, checks liveness and fresh
readiness, and calls no-state uninstall twice. It refuses to replace an existing
installation. Its `gui_activated: false` receipt is not GUI evidence. Final GUI QA
must launch this installed release artifact with disposable HOME/config/data;
`target/debug` and browser screenshots do not count.

The CI Linux release test uses installed development libraries when available.
The local fallback uses the same release-profile builder. Set
`CUTOKYO_RELEASE_PNPM_ROOT` to the unpacked pnpm distribution before running the
unchanged acceptance command on a host without WebKit development libraries.
Windows and macOS still need
successful jobs and clean-machine installation checks before a release manager
can claim those platforms passed.

## Verification and rotation

Before publication, independently check artifact hashes, updater signature
coverage, platform signatures, SBOM presence, provenance subjects, npm package
contents, and installed-binary smoke evidence. After publication, install from
the public release on clean supported platforms and compare bytes.

For key rotation:

1. stop releases;
2. revoke or expire the compromised platform credential with its issuer;
3. replace protected secrets through a two-person review;
4. update an updater public key only through the documented Tauri migration
   process—existing clients must not be stranded silently;
5. produce a new patch release;
6. document affected versions and recovery steps without disclosing private key
   material.

Follow [`recovery.md`](recovery.md) for partial or incorrect releases.

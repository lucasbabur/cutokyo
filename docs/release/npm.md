# CLI installation through npm

The npm package installs the **terminal CLI**. It does not install the Tauri desktop
application. Native targets are Linux x64 (glibc), Windows x64, and macOS ARM64/x64.

## What the workflow does

`.github/workflows/release-npm.yml` is manually dispatched, with three modes:

| Mode                | Result                                                                                                                                                                  |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dry-run` (default) | Build all four targets, run native and npm installation smoke checks, retain SBOMs, checksums, and provenance in `verified-npm-release`. No release or registry writes. |
| `publish-assets`    | Also upload the npm tarball, four native archives, and their SHA-256 files to an existing public GitHub release. Used for initial npm bootstrap.                        |
| `publish-npm`       | Download and re-test the existing release archives and npm tarball, then publish that exact tarball using OIDC and provenance. No rebuild or repack.                    |

Publishing requires the official repository, a stable version tag selected as the
workflow ref, matching Cargo/Tauri/workspace/UI versions, and an existing public
release for that tag. Branches and forks may only dry-run. A dry-run does not prove
that npm account authorization is configured. Native tests run on each target;
Linux additionally tests network/filesystem isolation using bubblewrap.

No platform signing secrets or `NPM_TOKEN` are required. Publication verifies the
wrapper's repository, version, native platform mappings and download URLs, then
compares every anonymous public download against its local hash. Existing assets
are never overwritten. The desktop and npm workflows share a tag concurrency group.
SBOMs, the build plan, smoke receipts and complete CLI inventory remain in the
`verified-npm-release` Actions artifact, with GitHub attestations.

## First publication

1. Merge this workflow into the official repository and run `dry-run` on `main`:

   ```bash
   gh workflow run release-npm.yml --repo lucasbabur/cutokyo --ref main -f mode=dry-run
   ```

2. Review the results. Prepare a **new version and immutable tag containing this
   workflow**, updating the repository's version files together. Do not move the
   existing `v0.1.0` tag. Create/publish its GitHub release through the release-owner
   process. A CLI-only release can be created without desktop certificates; label
   its scope accurately. The separate signed desktop workflow still enforces its
   own signing requirements.

3. Run `publish-assets` on that tag (replace `vX.Y.Z`):

   ```bash
   gh workflow run release-npm.yml --repo lucasbabur/cutokyo --ref vX.Y.Z -f mode=publish-assets
   ```

4. Download `verified-npm-release` from the successful run and verify its inventory
   from inside the downloaded artifact:

   ```bash
   sha256sum --check CLI-SHA256SUMS
   ```

   Review the tarball metadata and provenance. The npm account owner confirms that
   `cutokyo` is available, signs in with 2FA, then publishes **the downloaded tarball**:

   ```bash
   npm login
   npm publish ./global-installers/cutokyo-X.Y.Z.tgz --access public
   ```

   Do not pack the repository root: it is the desktop development workspace.
   This initial local publish does not claim npm's CI provenance; the build artifact
   retains its GitHub attestation. If the npm name is unavailable, stop and choose
   an authorized name in the cargo-dist configuration before rebuilding.

5. In the npm package settings, configure Trusted Publishing for GitHub owner
   `lucasbabur`, repository `cutokyo`, workflow filename `release-npm.yml`.
   Enable direct `npm publish` permission. Follow npm's current activation deadline
   and account requirements in its [official guide](https://docs.npmjs.com/trusted-publishers/).

## Later versions

Create the reviewed version/tag and public GitHub release. Populate its CLI assets
using `publish-assets` or the signed desktop release workflow, then dispatch
`publish-npm` on that tag. This mode reuses the already-published bytes, avoiding
collisions from rebuilding the same version. The workflow pins Node 22.22.3 and npm 11.5.1, which
support Trusted Publishing. No persistent npm token is stored in GitHub.

```bash
gh workflow run release-npm.yml --repo lucasbabur/cutokyo --ref vX.Y.Z -f mode=publish-npm
```

If publication fails after uploading assets, preserve the original run's artifacts
and rerun the failed publication job after correcting the cause. A full rebuild may
produce different bytes and will be refused if it collides with an existing asset.
See [recovery.md](recovery.md). Do not republish the bootstrap version.

After registry publication, validate on a clean supported system:

```bash
npm install --global cutokyo@X.Y.Z
cutokyo --json version
cutokyo --json doctor
```

`doctor` may report providers that have not been configured on a clean system;
check its structured output separately from successful installation/version.

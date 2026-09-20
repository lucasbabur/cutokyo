# Release recovery runbook

## Contents

- [Stop conditions](#stop-conditions)
- [Failure before publication](#failure-before-publication)
- [Partial publication](#partial-publication)
- [Bad but uncompromised release](#bad-but-uncompromised-release)
- [Credential compromise](#credential-compromise)
- [Database migration recovery](#database-migration-recovery)
- [Evidence and closure](#evidence-and-closure)

## Stop conditions

Stop the release immediately when any of these occurs:

- tracked source changes during a build/package command;
- a required signing secret is absent;
- cargo-dist plan, version, or tag disagree;
- an artifact lacks its expected checksum, SBOM entry, or provenance subject;
- npm launches a checkout or `target/` binary instead of its installed native
  package executable;
- a platform or updater signature is absent or invalid;
- package installation, launch, readiness, or uninstall smoke fails;
- publication starts from a manual dry-run or unreviewed ref.

Do not weaken a gate, regenerate evidence by hand, retag different bytes with the
same version, or upload an unsigned replacement under a signed asset name.

## Failure before publication

A failed manual dry-run has no remote recovery step. Preserve safe logs, source
snapshot manifests, exact command exits, cargo-dist plan, and artifact hashes.
Delete runner-temporary certificates and test artifacts. Fix the responsible
source/configuration in a new signed-off commit, rerun affected checks, then
rerun the complete dry-run.

A tag job can fail before publication after signing preflight and artifact jobs.
Confirm that npm has no such version and that GitHub has no public release; also
check for a draft created by a late publication-stage failure. If nothing was
published, delete any draft and delete the tag only under the repository's
authorized release policy, fix forward, and create a new reviewed tag. Prefer a
new patch version when consumers could have observed the tag.

## Partial publication

The workflow creates a draft GitHub release from the final, attested artifact set,
publishes the already-packed npm tarball, and only then makes the draft public.
It never rebuilds between these steps.

If draft creation fails, npm publication has not started. Preserve the exact
artifact set and diagnose GitHub permissions before an authorized retry.

If npm publication fails, keep the GitHub release as a draft and stop automated
retries. Determine whether npm accepted the version despite the reported failure.
If it did not, an authorized retry may publish the exact same tarball digest. If
npm accepted it, treat the version as immutable and follow the completion path
below.

If npm succeeds but making the GitHub draft public fails:

1. stop all automated retries;
2. verify the registry tarball digest against the locally attested artifact;
3. verify the draft still contains the exact checksummed release set;
4. do not overwrite or unpublish npm merely to reuse the version;
5. after human authorization, publish that existing draft without rebuilding or
   replacing any asset;
6. if exact completion is impossible, deprecate the npm version with a clear
   message and release a new patch version;
7. record which channels observed the partial state.

Never point an immutable npm version or an existing draft at newly rebuilt bytes
with different hashes.

## Bad but uncompromised release

Do not mutate signed release assets in place. Publish a new patch release that
contains the correction. Mark the affected version and failure mode clearly.
Where supported, prevent the updater manifest from recommending the bad version,
but retain historical checksums and provenance for auditability.

Users recover by stopping Cutokyo, preserving the active database/spool, and
installing a verified safe binary. An older binary must not open a newer database
schema. If the bad version migrated the database, restore only through the
digested online-backup recovery path or use a corrected forward-migrating
version; never copy a live `.db` over WAL state.

## Credential compromise

Treat platform and updater keys separately:

- revoke a compromised Apple Developer ID certificate through Apple;
- revoke/replace the Windows code-signing certificate with its issuer;
- rotate npm automation tokens and review registry audit history;
- rotate the Tauri updater private key only with an explicit public-key migration
  plan for existing installations;
- invalidate leaked GitHub credentials and review release/audit events.

Remove the workflow environment from service until all protected values are
replaced. Inventory releases signed during the exposure window. Publish a
security advisory with verifiable affected hashes; never include the leaked
secret itself in logs or evidence.

## Database migration recovery

Before every upgrade, retain a verified online backup and the previous binary.
If migration or integrity fails:

1. stop the writer and capture safe doctor output;
2. do not repeatedly reopen or manually edit the database;
3. verify the backup digest before staging restore;
4. retain the current failed copy and prior copy until restored integrity is
   `ok`;
5. restore through the application use case, not filesystem copy;
6. rerun schema, derive, FTS5, integrity, spool, and health checks;
7. keep liveness and readiness results separate.

A restore interruption must leave at least one known prior copy. A successful
unrelated write must not clear an integrity, backup, or quarantine degradation.

## Evidence and closure

Record the release tag and revision, actor/authorization, exact failed command
and exit, source-snapshot result, checksums, signing/notarization status, registry
state, affected platforms, and recovery decision. Keep private artifacts out of
public incident notes.

Close recovery only after:

- all required checks pass on one revision;
- the complete replacement artifact set is checksummed and attested;
- package-installed CLI and desktop smoke tests pass where available;
- npm and GitHub release bytes match the reviewed set;
- signing credentials and updater coverage are independently verified;
- support and release notes identify affected versions honestly.

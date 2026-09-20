export const meta = {
  name: 'cutokyo-finish',
  description: 'Finish Cutokyo v0.x: land the three reviewed candidates, then prove all twenty criteria on one revision',
  whenToUse: 'Run when C16, C17 and C19 candidate branches exist unmerged and the integrated acceptance still has to be proven.',
  phases: [
    { title: 'Candidates', detail: 'repair and independently review C16, C17 and C19 in parallel' },
    { title: 'Integration', detail: 'merge approved candidates and prove the integrated revision' },
    { title: 'Verification', detail: 'deterministic gates, real product QA, and Haiku JEV' },
    { title: 'Review', detail: 'independent judgment over all twenty criteria' },
    { title: 'Repair', detail: 'route verified defects to owners and reintegrate' },
    { title: 'Report', detail: 'assemble same-revision evidence' },
  ],
}

const ROOT = '/home/lucas/Developer/personal/cutokyo-community'
const FLEET = `${ROOT}/.claude/fleets/20260919-cutokyo-v01`
const OUT = `${ROOT}/.claude/fleets/20260920-cutokyo-finish`
const ACCEPT = `python3 ${ROOT}/tools/fleet/cutokyo-acceptance.py`
const GATES = `python3 ${ROOT}/tools/fleet/cutokyo-gates.py`
const DIFFGUARD = 'python3 /home/lucas/.claude/fleet-tools/diffguard/diffguard.py'

const REQUIRED_JEV_CASES = [
  'onboarding-empty-history',
  'search-detail-resume',
  'retention-delete-confirmation',
  'guard-proxy-coverage-language',
  'analysis-preview-cancel',
  'degraded-health-recovery',
  'mcp-plugin-inventory',
  'visual-keyboard-consistency',
]

const REQUIRED_QA_JOURNEYS = [
  'onboarding-setup-rollback',
  'empty-history',
  'populated-history',
  'search-resume-claude',
  'search-resume-codex',
  'search-resume-opencode',
  'retention-delete',
  'dashboard-reconciliation',
  'inventory',
  'guards',
  'analysis-consent-cancel-retry',
  'health-persistence-restart',
  'doctor-bundle-backup',
  'native-tauri-startup',
]

const COMMON = `You are working in Fleet 20260920-cutokyo-finish, which finishes Fleet 20260919-cutokyo-v01.
Repository: ${ROOT}
Acceptance contract: ${FLEET}/review.json — read the criterion, its pass condition, procedure and failure cases before editing anything.
Context: ${FLEET}/CONTEXT.md
Evidence root: ${OUT}/evidence
Acceptance runner: ${ACCEPT} run --criterion <ID> --root <dir> --json
Static gates: ${GATES}
Gate-evasion scan: ${DIFFGUARD}

Hard rules, in force for every action you take:
- review.json is read-only. Never edit, reorder, reword or reformat it, and never change a recorded command.
- Never weaken, skip, delete or work around a check to make it pass. If a check itself is genuinely invalid, stop and return done=false with an invalid-check-human gap that states the exact contradiction and the evidence for it, so a human decides.
- The acceptance runner treats a filtered cargo test that selects no test as failure (exit 86). That is a real defect, not a tooling artifact: add the missing test rather than changing the filter.
- Never push, tag, publish or release. Never read or copy /home/lucas/Developer/personal/cutokyo.
- Never inspect credentials, and never mutate real Claude Code, Codex or OpenCode configuration. Tests use disposable HOME, config and data roots.
- Browser or JEV evidence never substitutes for native Tauri or installed-harness evidence.
- Do not claim evidence you did not collect. Report a command's real output, including failures.
- Make atomic commits. Every commit message ends with:
Signed-off-by: Lucasbabur <lucasbabur@gmail.com>
Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
- Leave the main checkout at ${ROOT} untouched: work only in your own worktree.`

const BUILD_SCHEMA = {
  type: 'object',
  properties: {
    key: { type: 'string' },
    done: { type: 'boolean' },
    branch: { type: 'string' },
    revision: { type: 'string' },
    worktree: { type: 'string' },
    commits: { type: 'array', items: { type: 'string' } },
    files_changed: { type: 'array', items: { type: 'string' } },
    commands: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          command: { type: 'string' },
          exit_code: { type: 'integer' },
          summary: { type: 'string' },
        },
        required: ['command', 'exit_code', 'summary'],
      },
    },
    mutations: { type: 'array', items: { type: 'string' } },
    evidence: { type: 'array', items: { type: 'string' } },
    gaps: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
  required: ['key', 'done', 'branch', 'revision', 'worktree', 'commits', 'files_changed', 'commands', 'mutations', 'evidence', 'gaps', 'notes'],
}

const REVIEW_SCHEMA = {
  type: 'object',
  properties: {
    key: { type: 'string' },
    verdict: { type: 'string', enum: ['pass', 'fail'] },
    revision: { type: 'string' },
    contract_unchanged: { type: 'boolean' },
    ran_acceptance: { type: 'boolean' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          severity: { type: 'string', enum: ['high', 'medium', 'low'] },
          owner: { type: 'string' },
          defect: { type: 'string' },
          location: { type: 'string' },
          scenario: { type: 'string' },
          required_evidence: { type: 'string' },
          blocking: { type: 'boolean' },
        },
        required: ['id', 'severity', 'owner', 'defect', 'location', 'scenario', 'required_evidence', 'blocking'],
      },
    },
    personally_run: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' },
  },
  required: ['key', 'verdict', 'revision', 'contract_unchanged', 'ran_acceptance', 'findings', 'personally_run', 'summary'],
}

const INTEGRATION_SCHEMA = {
  type: 'object',
  properties: {
    done: { type: 'boolean' },
    revision: { type: 'string' },
    merged_branches: { type: 'array', items: { type: 'string' } },
    conflicts: { type: 'array', items: { type: 'string' } },
    commands: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          command: { type: 'string' },
          exit_code: { type: 'integer' },
          summary: { type: 'string' },
        },
        required: ['command', 'exit_code', 'summary'],
      },
    },
    criteria_run: { type: 'array', items: { type: 'string' } },
    gaps: { type: 'array', items: { type: 'string' } },
  },
  required: ['done', 'revision', 'merged_branches', 'conflicts', 'commands', 'criteria_run', 'gaps'],
}

const CHECK_SCHEMA = {
  type: 'object',
  properties: {
    selftests_ok: { type: 'boolean' },
    all_passed: { type: 'boolean' },
    revision: { type: 'string' },
    results: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          status: { type: 'string', enum: ['pass', 'fail', 'void', 'warn'] },
          owner: { type: 'string' },
          command: { type: 'string' },
          exit_code: { type: 'integer' },
          diagnostic: { type: 'string' },
          evidence: { type: 'string' },
        },
        required: ['key', 'status', 'owner', 'command', 'exit_code', 'diagnostic', 'evidence'],
      },
    },
  },
  required: ['selftests_ok', 'all_passed', 'revision', 'results'],
}

const QA_SCHEMA = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['pass', 'fail'] },
    revision: { type: 'string' },
    native_app_tested: { type: 'boolean' },
    installed_harnesses: { type: 'array', items: { type: 'string' } },
    journeys: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          status: { type: 'string', enum: ['pass', 'fail', 'blocked'] },
          evidence: { type: 'array', items: { type: 'string' } },
          summary: { type: 'string' },
        },
        required: ['key', 'status', 'evidence', 'summary'],
      },
    },
    failures: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          owner: { type: 'string' },
          severity: { type: 'string' },
          defect: { type: 'string' },
          reproduction: { type: 'string' },
          evidence: { type: 'string' },
        },
        required: ['key', 'owner', 'severity', 'defect', 'reproduction', 'evidence'],
      },
    },
    gaps: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' },
  },
  required: ['verdict', 'revision', 'native_app_tested', 'installed_harnesses', 'journeys', 'failures', 'gaps', 'summary'],
}

const JEV_SCHEMA = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['pass', 'fail'] },
    revision: { type: 'string' },
    validated: { type: 'boolean' },
    screenshots_read: { type: 'boolean' },
    batch: { type: 'string' },
    report: { type: 'string' },
    cases: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          status: { type: 'string', enum: ['PASS', 'FAIL', 'ERROR', 'BLOCKED'] },
          screenshots: { type: 'array', items: { type: 'string' } },
          summary: { type: 'string' },
        },
        required: ['key', 'status', 'screenshots', 'summary'],
      },
    },
    failures: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          owner: { type: 'string' },
          defect: { type: 'string' },
          reproduction: { type: 'string' },
          evidence: { type: 'string' },
        },
        required: ['key', 'owner', 'defect', 'reproduction', 'evidence'],
      },
    },
    summary: { type: 'string' },
  },
  required: ['verdict', 'revision', 'validated', 'screenshots_read', 'batch', 'report', 'cases', 'failures', 'summary'],
}

const FINAL_REVIEW_SCHEMA = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['pass', 'fail'] },
    revision: { type: 'string' },
    criteria: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          verdict: { type: 'string', enum: ['pass', 'fail'] },
          basis: { type: 'string' },
        },
        required: ['id', 'verdict', 'basis'],
      },
    },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          owner: { type: 'string' },
          severity: { type: 'string' },
          defect: { type: 'string' },
          location: { type: 'string' },
          reproduction: { type: 'string' },
        },
        required: ['key', 'owner', 'severity', 'defect', 'location', 'reproduction'],
      },
    },
    gaps: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' },
  },
  required: ['verdict', 'revision', 'criteria', 'findings', 'gaps', 'summary'],
}

const OWNERS = {
  core: 'cutokyo-core-builder',
  'cli-ops': 'cutokyo-cli-ops-builder',
  desktop: 'cutokyo-desktop-builder',
  extensions: 'cutokyo-extension-builder',
  'harness-claude': 'cutokyo-harness-builder',
  'harness-codex': 'cutokyo-harness-builder',
  'harness-opencode': 'cutokyo-harness-builder',
  harness: 'cutokyo-harness-builder',
  integration: 'cutokyo-architect',
  architect: 'cutokyo-architect',
}

const CANDIDATES = [
  {
    key: 'C16',
    criterion: 'C16',
    branch: 'claude-worktree/agent-a9b63d7df3fb5e31c-709e03773027',
    head: '66deba6e16bb22226681ed9f5dcb95d3b03dba00',
    agentType: 'cutokyo-cli-ops-builder',
    owner: 'cli-ops',
    repaired: true,
    brief: `C16 diagnostics and persisted health. An earlier review rejected the first commit on this branch and the follow-up commit 66deba6 claims to fix all of it: bounded metadata-only source logs with an exact field allowlist, panic thread names classified rather than persisted, crash clearing bound to a published bundle receipt including a replacement-record probe, the exact test driving a binary built from the real CLI main.rs instead of newly public internals, recursive decoded-JSON inspection so escaped Windows paths cannot evade the leak scan, health parity across core use cases, CLI doctor JSON and the desktop binding projection, and bounded health reads with every growing history table removed. Verify each of those claims against the code rather than the report.`,
  },
  {
    key: 'C19',
    criterion: 'C19',
    branch: 'claude-worktree/agent-afe95f516e67337e3-630d5b1f861e',
    head: 'ff089c23ef4e651b84f94266f281dfe5b82156d6',
    agentType: 'cutokyo-cli-ops-builder',
    owner: 'cli-ops',
    repaired: false,
    brief: `C19 release artifacts and supply chain. Commit 7468bb4 was rejected by independent review and commit ff089c2 is an unreviewed, possibly incomplete repair pass. The nine findings were: Windows CI used Bash '$CARGO_DIST_VERSION' under PowerShell; the source snapshot could miss a rewrite that is restored before exit, accepted evidence and capture paths inside the worktree, wrote evidence after the comparison, and could not detect a replaced split capture or a moved HEAD/index; updater signature validation accepted any non-empty whitespace-free string instead of decoding and verifying a real Tauri/minisign signature; the npm smoke inherited the whole environment and only hinted npm offline rather than enforcing egress denial, while hardcoding a network_mode claim the test then asserted; no exact test installed, started or uninstalled a real built Linux package, and the manifest test fabricated the package-smoke receipt; the npm wrapper test used a handmade debug archive instead of the archive cargo-dist actually emits and ran only version, not doctor; SBOM and provenance validation accepted a skeletal CycloneDX document and an empty-subject attestation, and workflow evidence was raw substring matching; the final SHA256SUMS omitted latest.json and release-manifest.json while the documentation promised a complete inventory; macOS updater architecture was inferred from an architecture-free .app filename and defaulted to x86-64. Establish precisely which of these ff089c2 already fixes before doing anything, then finish the rest.`,
  },
  {
    key: 'C17',
    criterion: 'C17',
    branch: 'fix/c17-native-core-invoke',
    head: '85e9229720f166f6600afbd43ff890a5b03dfb3f',
    agentType: 'cutokyo-desktop-builder',
    owner: 'desktop',
    repaired: false,
    brief: `C17 native desktop. Commit 85e9229 made the native WebdriverIO journey genuinely pass by embedding the frontend in debug builds and installing the official WDIO command bridge under the native-e2e feature, and independent review confirmed the Rust production isolation is sound. It then rejected the commit for six reasons: the journey drives an unpackaged target/debug binary, so no installed or bundled Linux package is ever started; native E2E overwrites the shared production ui/dist, so a later direct cargo release build can embed stale bridge assets even without the Rust feature; concurrent native runs share an embedded port and can share a test root, and the user onComplete deletes the test root before the service kills the application; the geometry assertion only proves the configured 900x700 minimum while the screenshot is named 1280x800; the fake resume harness publishes the audit pathname before its contents are complete while the test waits only for existence; and fixture seeding is gated on debug_assertions alone rather than the native-e2e feature plus validated test mode. The supported environment for the native run is the local Docker image cutokyo-tauri-qa:2.11.4 with Xvfb and D-Bus; the reviewer's own host lacks WebKit and DBus development packages.`,
  },
]

function worktreeFor(key) {
  return `/tmp/cutokyo-finish-worktrees/${key.toLowerCase()}`
}

function workspaceRules(candidate) {
  return `Your workspace is a dedicated worktree you create yourself, so nothing else can disturb it:

    git -C ${ROOT} worktree add ${worktreeFor(candidate.key)} ${candidate.branch}

If that path already exists from an earlier attempt, reuse it after confirming it is on ${candidate.branch} and clean. Work only inside it, and commit on ${candidate.branch}. Never edit the main checkout at ${ROOT}. Note that /tmp is a tmpfs that does not survive a reboot, so commit meaningful progress as you go rather than holding a large uncommitted change.`
}

function buildPrompt(candidate, round, priorFindings) {
  const findings = priorFindings && priorFindings.length
    ? `\n\nAn independent reviewer rejected the current revision. Repair exactly these findings, and nothing outside your ownership:\n${JSON.stringify(priorFindings, null, 2)}`
    : ''
  return `${COMMON}

You own criterion ${candidate.criterion} on branch ${candidate.branch} (currently ${candidate.head}). Repair round ${round}.

${candidate.brief}
${findings}

${workspaceRules(candidate)}

What finishing means here:
1. Read the ${candidate.criterion} entry in ${FLEET}/review.json in full, including pass_condition, procedure and failure_cases.
2. Fix the substance, not the symptom. Prefer the simplest implementation that fully meets the requirement, and prefer an established, well-maintained library over custom code when one fits; say why if you write custom code.
3. Prove every load-bearing assertion with a deliberate mutation: break the production behaviour, show the exact named test fails for the intended reason, restore it, show it passes. Never commit a mutation.
4. Run the criterion through the acceptance runner, which refuses zero-test filters:
   ${ACCEPT} run --criterion ${candidate.criterion} --root ${worktreeFor(candidate.key)} --json
   It must exit 0. Exit 86 means a required test does not exist; write it.
5. Run the checks your change can affect, at minimum cargo fmt --all --check, warning-denying clippy over the targets you touched, the suites your change touches, ${GATES} selftest and all, and ${DIFFGUARD} scan against your base.
6. Commit atomically, leave the worktree clean, and report the final revision.

Report honestly: a gap you disclose is cheap, a gap a reviewer finds is not.`
}

function reviewPrompt(candidate, revision, worktree, buildResult) {
  return `${COMMON}

Independently review criterion ${candidate.criterion} at revision ${revision} on branch ${candidate.branch}. You are the acceptance gate: the builder's report is context, never evidence.

${candidate.brief}

Builder handoff:
${JSON.stringify(buildResult, null, 2)}

Inspect the work yourself in the builder's worktree at ${worktree}, or in a read-only worktree of your own if you prefer isolation. Do not edit, commit, or repair anything anywhere.

Your job:
1. Confirm review.json is byte-identical to the version on main, and that no acceptance command was altered.
2. Read every changed file. Judge whether the criterion's pass_condition is genuinely met, not merely whether tests are green.
3. Hunt specifically for: assertions that cannot fail, tests that exercise helpers instead of the shipped artifact, production API widened only to make a test possible, privacy or supply-chain regressions, platform-specific breakage, races, and evidence that was fabricated rather than observed.
4. Run the acceptance runner yourself and record its real output:
   ${ACCEPT} run --criterion ${candidate.criterion} --root <the worktree> --json
5. Where a claim is load-bearing and cheap to test, test it. Say plainly which commands you ran and which claims you could not verify and why.

Return verdict pass only when you would be comfortable defending this revision to a hostile reviewer. Every finding needs an exact file:line location and a concrete failure scenario. Mark blocking=true for anything that must be fixed before merge.`
}

phase('Candidates')
log(`Three candidates: ${CANDIDATES.map((c) => c.key).join(', ')}`)

const candidateResults = await pipeline(CANDIDATES, async (candidate) => {
  const maxRounds = 3
  let round = 0
  let findings = []
  let build = null
  let review = null

  // A repaired candidate is reviewed first; an unrepaired one is built first.
  if (candidate.repaired) {
    build = {
      key: candidate.key,
      done: true,
      branch: candidate.branch,
      revision: candidate.head,
      worktree: worktreeFor(candidate.key),
      commits: [candidate.head],
      files_changed: [],
      commands: [],
      mutations: [],
      evidence: ['Branch was repaired before this fleet; it goes straight to independent review.'],
      gaps: [],
      notes: 'Carried in as already repaired.',
    }
    review = await agent(
      reviewPrompt(candidate, candidate.head, worktreeFor(candidate.key), build),
      { agentType: 'cutokyo-technical-reviewer', label: `review:${candidate.key}:r0`, phase: 'Candidates', schema: REVIEW_SCHEMA },
    )
    if (review && review.verdict === 'pass') {
      log(`${candidate.key}: approved at ${candidate.head} without further repair`)
      return { candidate, build, review, approved: true, rounds: 0 }
    }
    findings = (review?.findings || []).filter((f) => f.blocking)
    log(`${candidate.key}: review found ${findings.length} blocking finding(s)`)
  }

  while (round < maxRounds) {
    round += 1
    build = await agent(
      buildPrompt(candidate, round, findings),
      { agentType: candidate.agentType, label: `build:${candidate.key}:r${round}`, phase: 'Candidates', schema: BUILD_SCHEMA },
    )
    if (!build || !build.done) {
      log(`${candidate.key}: builder stopped in round ${round}`)
      return { candidate, build, review, approved: false, rounds: round, halted: 'builder did not reach readiness' }
    }
    review = await agent(
      reviewPrompt(candidate, build.revision, build.worktree || worktreeFor(candidate.key), build),
      { agentType: 'cutokyo-technical-reviewer', label: `review:${candidate.key}:r${round}`, phase: 'Candidates', schema: REVIEW_SCHEMA },
    )
    if (!review) {
      return { candidate, build, review, approved: false, rounds: round, halted: 'review did not return a verdict' }
    }
    if (review.verdict === 'pass') {
      log(`${candidate.key}: approved at ${build.revision} after ${round} repair round(s)`)
      return { candidate, build, review, approved: true, rounds: round }
    }
    findings = review.findings.filter((f) => f.blocking)
    log(`${candidate.key}: round ${round} rejected with ${findings.length} blocking finding(s)`)
  }

  return { candidate, build, review, approved: false, rounds: maxRounds, halted: 'same candidate failed three review rounds' }
})

const usable = candidateResults.filter(Boolean)
const approved = usable.filter((r) => r.approved)
const rejected = usable.filter((r) => !r.approved)

log(`Candidates approved: ${approved.map((r) => r.candidate.key).join(', ') || 'none'}`)
if (rejected.length) {
  log(`Candidates NOT approved: ${rejected.map((r) => r.candidate.key).join(', ')}`)
}

if (!approved.length) {
  return { accepted: false, halted: 'no candidate reached an independent pass', candidates: usable }
}

phase('Integration')
let integration = await agent(
  `${COMMON}

Merge the independently approved candidate branches into main and prove the integrated revision.

Approved candidates:
${JSON.stringify(approved.map((r) => ({ key: r.candidate.key, branch: r.candidate.branch, revision: r.build.revision, review: r.review.summary })), null, 2)}

${rejected.length ? `Not approved, so NOT merged, and their criteria will be reported as unmet:\n${JSON.stringify(rejected.map((r) => ({ key: r.candidate.key, halted: r.halted, summary: r.review?.summary || 'no verdict' })), null, 2)}` : 'Every candidate was approved.'}

Work directly in ${ROOT} on main, which is currently clean. Merge each approved branch in an order that minimises conflict, resolving only mechanical collisions: preserve every unit's behaviour and never drop a test. These branches all touch tests/e2e.rs and several touch crates/cutokyo-cli, so expect real conflicts and resolve them by keeping both sides' substance.

After merging, prove the integrated revision:
- ${ACCEPT} run --criterion <ID> --root ${ROOT} --json for every criterion the merge could affect, at minimum C12, C16, C17, C18, C19 and C20. Each must exit 0. Exit 86 means a test was lost in the merge.
- cargo test -p cutokyo-integration-tests --test e2e -- --list, and confirm every exact acceptance filter name appears exactly once.
- ${GATES} selftest and ${GATES} all --root ${ROOT}.
- ${DIFFGUARD} selftest and a scan against the pre-merge revision.
Save logs under ${OUT}/evidence/integration/.

Return done=false if a branch had to be dropped, a required check cannot run, or a merge conflict hides a real product decision.`,
  { agentType: 'cutokyo-architect', label: 'integrate', phase: 'Integration', schema: INTEGRATION_SCHEMA },
)

if (!integration || !integration.done) {
  return { accepted: false, halted: 'integration failed', candidates: usable, integration }
}

let accepted = false
let round = 0
let gates = null
let qa = null
let jev = null
let review = null
const history = []

while (!accepted && round < 4) {
  round += 1

  phase('Verification')
  const verified = await parallel([
    () => agent(
      `${COMMON}

Verification round ${round} against integrated revision ${integration.revision} in ${ROOT}.

Run the deterministic acceptance for all twenty criteria through the runner, which refuses zero-test filters:
  ${ACCEPT} run --criterion C01 --root ${ROOT} --json
through C20, plus ${GATES} selftest first and then ${GATES} all --root ${ROOT}, and ${DIFFGUARD} selftest.

Report each criterion as its own result entry keyed by its id, with the runner's real exit code. Exit 86 is a fail, not a pass: it means a named acceptance test does not exist. Treat a criterion you could not run because of a missing local dependency as void, name the dependency, and never report it as pass. Save every log under ${OUT}/evidence/round-${round}/commands/. Do not install, repair or edit anything.`,
      { agentType: 'cutokyo-gatekeeper', label: `gates:r${round}`, phase: 'Verification', schema: CHECK_SCHEMA },
    ),
    () => agent(
      `${COMMON}

Verification round ${round} against integrated revision ${integration.revision}.

Use Cutokyo as a real product, not as a test suite. Load the cutokyo-runtime-qa skill and follow it. Exercise the native desktop application and the CLI against fake worlds and safe read-only probes of installed harnesses, with disposable config and data roots throughout. The native desktop runs in the local Docker image cutokyo-tauri-qa:2.11.4 under Xvfb and D-Bus.

Return exactly one entry for each required journey key, each backed by direct evidence you captured: ${REQUIRED_QA_JOURNEYS.join(', ')}.

Save screenshots and journey logs under ${OUT}/evidence/round-${round}/qa/. Judge the visual design and the honesty of the product's language as well as its behaviour: absent must not be displayed as zero, and an unavailable capability must say so rather than inventing a result. A browser-only observation presented as native desktop coverage is a failure, not a pass. native_app_tested must be true only if you actually drove the native application.`,
      { agentType: 'cutokyo-product-qa', label: `qa:r${round}`, phase: 'Verification', schema: QA_SCHEMA },
    ),
    () => agent(
      `${COMMON}

Verification round ${round} against integrated revision ${integration.revision}.

Load and follow the installed jev-qa skill at /home/lucas/.codex/skills/jev-qa/SKILL.md. Start the integrated browser fixture mode with isolated Cutokyo config and data at the batch's 127.0.0.1:4173 URL. Validate ${FLEET}/jev-cases.yaml, then run that exact batch unchanged once and save its report under ${OUT}/evidence/round-${round}/jev/.

It defines exactly one independent case for each required key: ${REQUIRED_JEV_CASES.join(', ')}.

Read every resulting screenshot into context, including PASS, ERROR and BLOCKED outcomes. JEV is supplemental browser evidence and must never be presented as native Tauri or installed-harness coverage. Return ERROR or BLOCKED as a failure with routable findings. Never repair the product or lower an expectation.`,
      { agentType: 'cutokyo-jev-checker', label: `jev:r${round}`, phase: 'Verification', schema: JEV_SCHEMA },
    ),
  ])

  gates = verified[0]
  qa = verified[1]
  jev = verified[2]

  if (!gates || gates.selftests_ok === false) {
    return { accepted: false, halted: 'verifier selftest failed, so every downstream verdict is void', round, candidates: usable, integration, gates }
  }

  phase('Review')
  review = await agent(
    `${COMMON}

Final independent judgment of integrated revision ${integration.revision}, round ${round}.

Judge every criterion C01 through C20 in ${FLEET}/review.json against the delivered artifact. Inspect the repository yourself; the reports below are context, never evidence. Where a verdict depends on a claim you can cheaply test, test it.

Deterministic results:
${JSON.stringify(gates, null, 2)}

Product QA:
${JSON.stringify(qa, null, 2)}

Browser JEV:
${JSON.stringify(jev, null, 2)}

${rejected.length ? `These criteria were never landed and must be judged fail with that stated plainly:\n${JSON.stringify(rejected.map((r) => r.candidate.criterion), null, 2)}` : ''}

Judge in particular: whether the product honestly represents what it does and does not know, whether provenance and the clean-room boundary hold, whether privacy and egress claims match the code, whether the plugin, MCP and proxy contracts are sound, and whether release and signing claims are honest about what was actually proven locally versus what only CI would produce.

Return verdict pass only if every criterion passes on this one revision.`,
    { agentType: 'cutokyo-technical-reviewer', label: `review:r${round}`, phase: 'Review', schema: FINAL_REVIEW_SCHEMA },
  )

  const failures = []
  for (const result of gates.results || []) {
    if (result.status === 'fail' || result.status === 'void') {
      failures.push({ key: result.key, owner: result.owner, severity: 'high', defect: result.diagnostic, location: result.command, reproduction: result.command })
    }
  }
  for (const failure of qa?.failures || []) {
    failures.push({ key: `qa-${failure.key}`, owner: failure.owner, severity: failure.severity, defect: failure.defect, location: failure.evidence, reproduction: failure.reproduction })
  }
  for (const failure of jev?.failures || []) {
    failures.push({ key: `jev-${failure.key}`, owner: failure.owner, severity: 'high', defect: failure.defect, location: failure.evidence, reproduction: failure.reproduction })
  }
  for (const finding of review?.findings || []) {
    failures.push(finding)
  }

  const gatesOk = gates.all_passed === true
  const qaOk = qa?.verdict === 'pass' && qa.native_app_tested === true &&
    REQUIRED_QA_JOURNEYS.every((key) => (qa.journeys || []).some((j) => j.key === key && j.status === 'pass'))
  const jevOk = jev?.verdict === 'pass' && jev.validated === true && jev.screenshots_read === true &&
    REQUIRED_JEV_CASES.every((key) => (jev.cases || []).some((c) => c.key === key && c.status === 'PASS'))
  const reviewOk = review?.verdict === 'pass' && (review.criteria || []).length === 20 &&
    (review.criteria || []).every((c) => c.verdict === 'pass')

  history.push({ round, revision: integration.revision, gates: gatesOk, qa: qaOk, jev: jevOk, review: reviewOk, failures: failures.length })
  log(`round ${round}: gates=${gatesOk} qa=${qaOk} jev=${jevOk} review=${reviewOk} findings=${failures.length}`)

  accepted = gatesOk && qaOk && jevOk && reviewOk
  if (accepted) break

  if (!failures.length) {
    return { accepted: false, halted: 'a verdict was short of complete but produced no routable finding', round, history, gates, qa, jev, review }
  }

  if (round >= 4) break

  phase('Repair')
  const byOwner = {}
  for (const failure of failures) {
    const owner = OWNERS[failure.owner] ? failure.owner : 'integration'
    if (!byOwner[owner]) byOwner[owner] = []
    byOwner[owner].push(failure)
  }

  const repairs = await parallel(Object.keys(byOwner).map((owner) => () => agent(
    `${COMMON}

Repair round ${round}. You own these verified findings against integrated revision ${integration.revision}:
${JSON.stringify(byOwner[owner], null, 2)}

Create your own worktree from main and work there:
  git -C ${ROOT} worktree add /tmp/cutokyo-finish-worktrees/repair-${round}-${owner} main

Fix the defect the finding describes. If a finding is actually a faulty check, a conflicting requirement or a missing dependency, do not edit or work around the check: return done=false with an invalid-check-human gap and the exact contradiction. Mutation-prove each repair, rerun the affected criteria through ${ACCEPT}, commit atomically, and report your branch and revision.`,
    { agentType: OWNERS[owner], label: `repair:${owner}:r${round}`, phase: 'Repair', schema: BUILD_SCHEMA },
  )))

  const usableRepairs = repairs.filter(Boolean).filter((r) => r.done && r.branch)
  if (!usableRepairs.length) {
    return { accepted: false, halted: 'no repair unit reached merge readiness', round, history, failures, repairs }
  }

  integration = await agent(
    `${COMMON}

Merge the round-${round} repair branches into main in ${ROOT} and fix only merge artifacts.

Repairs:
${JSON.stringify(usableRepairs, null, 2)}

Then rerun every criterion affected by these changes through ${ACCEPT}, plus ${GATES} all and a diffguard scan. Return done=false if any repair had to be dropped.`,
    { agentType: 'cutokyo-architect', label: `integrate:repair:r${round}`, phase: 'Integration', schema: INTEGRATION_SCHEMA },
  )

  if (!integration || !integration.done) {
    return { accepted: false, halted: 'repair integration failed', round, history, repairs: usableRepairs, integration }
  }
}

phase('Report')
const report = await agent(
  `${COMMON}

Assemble the final evidence report for revision ${integration.revision}. Write it to ${OUT}/report.md and a self-contained page at ${OUT}/index.html.

Accepted: ${accepted}

Candidates:
${JSON.stringify(usable.map((r) => ({ key: r.candidate.key, approved: r.approved, rounds: r.rounds, revision: r.build?.revision, review: r.review?.summary, halted: r.halted })), null, 2)}

Integration:
${JSON.stringify(integration, null, 2)}

Deterministic results:
${JSON.stringify(gates, null, 2)}

Product QA:
${JSON.stringify(qa, null, 2)}

Browser JEV:
${JSON.stringify(jev, null, 2)}

Technical review:
${JSON.stringify(review, null, 2)}

Round history:
${JSON.stringify(history, null, 2)}

The report states, for each of the twenty criteria, its verdict, the exact command or observation that establishes it, the revision it was established at, and the environment. It embeds or links the native desktop screenshots and the QA and JEV screenshots by path, and it distinguishes what was proven locally on Linux from what only CI or a real release would establish. It lists every remaining gap plainly. Do not overstate: if something was not run, say it was not run. Return the two file paths and a one-paragraph summary.`,
  { agentType: 'cutokyo-architect', label: 'report', phase: 'Report', schema: {
    type: 'object',
    properties: {
      report_path: { type: 'string' },
      html_path: { type: 'string' },
      revision: { type: 'string' },
      summary: { type: 'string' },
      gaps: { type: 'array', items: { type: 'string' } },
    },
    required: ['report_path', 'html_path', 'revision', 'summary', 'gaps'],
  } },
)

return {
  accepted,
  revision: integration.revision,
  candidates: usable.map((r) => ({ key: r.candidate.key, approved: r.approved, rounds: r.rounds, revision: r.build?.revision, halted: r.halted })),
  integration,
  gates,
  qa,
  jev,
  review,
  history,
  report,
}

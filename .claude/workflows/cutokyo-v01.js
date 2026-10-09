export const meta = {
  name: 'cutokyo-v01',
  description: 'Build and independently verify a usable local-first Cutokyo v0.x across Claude Code, Codex, and OpenCode',
  whenToUse: 'Run after approving Fleet 20260919-cutokyo-v01 from an empty or foundation-only Cutokyo repository.',
  phases: [
    { title: 'Foundation', detail: 'contracts, schemas, workspace, legal baseline, verifier selftests' },
    { title: 'Core', detail: 'SQLite, spool ingestion, projections, search, health, backups' },
    { title: 'Core merge', detail: 'merge and prove the durable vertical slice before fan-out' },
    { title: 'Feature build', detail: 'three harnesses, extensions, CLI/ops, and desktop in parallel' },
    { title: 'Integration', detail: 'merge owned branches and run integrated checks' },
    { title: 'Verification', detail: 'deterministic gates, real product QA, and Haiku JEV browser checks' },
    { title: 'Review', detail: 'independent requirement, architecture, security, and evidence judgment' },
    { title: 'Repair', detail: 'route verified defects to owners and reintegrate' },
    { title: 'Report', detail: 'assemble evidence and render the final review' },
  ],
}

const ROOT = '/home/lucas/Developer/personal/cutokyo-community'
const FLEET = `${ROOT}/.claude/fleets/20260919-cutokyo-v01`
const GATES = `${ROOT}/tools/fleet/cutokyo-gates.py`
const DIFFGUARD = '/home/lucas/.claude/fleet-tools/diffguard/diffguard.py'
const REQUIRED_CRITERIA = Array.from({ length: 20 }, (_, index) => `C${String(index + 1).padStart(2, '0')}`)
const REQUIRED_QA_JOURNEYS = [
  'onboarding-setup-rollback',
  'uninstall-never-activated',
  'restore-no-state-repeat',
  'restore-partial-concurrent-edit',
  'settings-contract-preservation',
  'empty-history',
  'populated-history',
  'search-resume-claude',
  'search-resume-codex',
  'search-resume-opencode',
  'retention-delete',
  'dashboard-reconciliation',
  'inventory',
  'mcp-failure-isolation',
  'plugin-verification',
  'guards',
  'proxy-consent-degraded',
  'analysis-consent-cancel-retry',
  'health-lock-quarantine',
  'health-persistence-restart',
  'health-current-lifetime-large-history',
  'doctor-bundle-backup',
  'backup-tamper-restore',
  'artifact-source-immutability',
  'updater-choice',
  'accessibility-keyboard',
  'native-tauri-startup',
]

const REQUIRED_JEV_CASES = [
  'onboarding-empty-history',
  'search-detail-resume',
  'retention-delete-confirmation',
  'guard-proxy-coverage-language',
  'degraded-health-recovery',
  'mcp-plugin-inventory',
  'visual-keyboard-consistency',
]

const BUILD_SCHEMA = {
  type: 'object',
  properties: {
    unit: { type: 'string' },
    done: { type: 'boolean' },
    branch: { type: 'string' },
    revision: { type: 'string' },
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
    evidence: { type: 'array', items: { type: 'string' } },
    gaps: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
  required: ['unit', 'done', 'branch', 'revision', 'commits', 'files_changed', 'commands', 'evidence', 'gaps', 'notes'],
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
    routed_findings: { type: 'array', items: { type: 'string' } },
    gaps: { type: 'array', items: { type: 'string' } },
  },
  required: ['done', 'revision', 'merged_branches', 'conflicts', 'commands', 'routed_findings', 'gaps'],
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
          name: { type: 'string' },
          status: { type: 'string', enum: ['pass', 'fail', 'blocked'] },
          evidence: { type: 'array', items: { type: 'string' } },
          summary: { type: 'string' },
        },
        required: ['key', 'name', 'status', 'evidence', 'summary'],
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
    batch: { type: 'string' },
    report: { type: 'string' },
    validated: { type: 'boolean' },
    screenshots_read: { type: 'boolean' },
    cases: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          status: { type: 'string', enum: ['PASS', 'ERROR', 'BLOCKED'] },
          evidence: { type: 'array', items: { type: 'string' } },
          screenshots: { type: 'array', items: { type: 'string' } },
          summary: { type: 'string' },
        },
        required: ['key', 'status', 'evidence', 'screenshots', 'summary'],
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
  required: ['verdict', 'revision', 'batch', 'report', 'validated', 'screenshots_read', 'cases', 'failures', 'gaps', 'summary'],
}

const REVIEW_SCHEMA = {
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
          verdict: { type: 'string', enum: ['pass', 'fail', 'undetermined'] },
          citation: { type: 'string' },
          note: { type: 'string' },
        },
        required: ['id', 'verdict', 'citation', 'note'],
      },
    },
    failures: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          owner: { type: 'string' },
          kind: { type: 'string', description: 'product | check | conflict | dependency' },
          defect: { type: 'string' },
          location: { type: 'string' },
          reproduction: { type: 'string' },
        },
        required: ['key', 'owner', 'kind', 'defect', 'location', 'reproduction'],
      },
    },
    gaps: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' },
  },
  required: ['verdict', 'revision', 'criteria', 'failures', 'gaps', 'summary'],
}

const TRIAGE_SCHEMA = {
  type: 'object',
  properties: {
    invalid_check: { type: 'boolean' },
    revision: { type: 'string' },
    assignments: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          owner: { type: 'string' },
          diagnosis: { type: 'string' },
        },
        required: ['key', 'owner', 'diagnosis'],
      },
    },
    resolved: { type: 'array', items: { type: 'string' } },
    unresolved: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
  required: ['invalid_check', 'revision', 'assignments', 'resolved', 'unresolved', 'notes'],
}

const REPORT_SCHEMA = {
  type: 'object',
  properties: {
    revision: { type: 'string' },
    report: { type: 'string' },
    review: { type: 'string' },
    rendered: { type: 'boolean' },
    opened: { type: 'boolean' },
    summary: { type: 'string' },
  },
  required: ['revision', 'report', 'review', 'rendered', 'opened', 'summary'],
}

const roleAssignments = {
  core: {
    agentType: 'cutokyo-core-builder',
    prompt: 'Repair only durable domain/store/ingest/config/search/health defects assigned below.',
  },
  'harness-claude': {
    agentType: 'cutokyo-harness-builder',
    prompt: 'Repair only Claude Code adapter/setup/fixture/resume/coverage defects assigned below.',
  },
  'harness-codex': {
    agentType: 'cutokyo-harness-builder',
    prompt: 'Repair only Codex adapter/setup/fixture/resume/coverage defects assigned below.',
  },
  'harness-opencode': {
    agentType: 'cutokyo-harness-builder',
    prompt: 'Repair only OpenCode adapter/setup/fixture/resume/coverage defects assigned below.',
  },
  extensions: {
    agentType: 'cutokyo-extension-builder',
    prompt: 'Repair only plugin/MCP/guard/proxy/AI-analysis defects assigned below.',
  },
  'cli-ops': {
    agentType: 'cutokyo-cli-ops-builder',
    prompt: 'Repair only CLI/config/diagnostics/logging/distribution/docs/CI defects assigned below.',
  },
  desktop: {
    agentType: 'cutokyo-desktop-builder',
    prompt: 'Repair only Tauri/UI/dashboard/accessibility/journey defects assigned below.',
  },
  integration: {
    agentType: 'cutokyo-architect',
    prompt: 'Repair only shared contracts, integration wiring, check defects, or evidence orchestration assigned below.',
  },
}

function commonAssignment(unit) {
  return `You are unit ${unit} in Fleet 20260919-cutokyo-v01.
Repository: ${ROOT}
Authoritative context: ${FLEET}/CONTEXT.md
Acceptance contract: ${FLEET}/review.json — read your criteria, failure cases, procedures, and exact commands before editing.
Evidence root: ${FLEET}/evidence
Static gate: python3 ${GATES}
Gate-evasion scan: python3 ${DIFFGUARD}
Repair owner keys: core, harness-claude, harness-codex, harness-opencode, extensions, cli-ops, desktop, integration.
Clean-room boundary: do not read or copy /home/lucas/Developer/personal/cutokyo; the independently worded observations you may use are already in CONTEXT.md and the provenance ledger.
Check skepticism: if an acceptance check, fixture expectation, or verifier appears invalid, do not edit, weaken, or work around it. Return done=false with an invalid-check-human gap, the exact contradiction, and evidence so the run stops for the human's decision.

Work only in the destination the harness gives you. Finish behavior, tests, documentation, and evidence in your ownership. Make atomic commits with Signed-off-by and the required Claude co-author trailer. Never push, publish, weaken a gate, skip a failing test, or call a scaffold complete.`
}

phase('Foundation')
const resumedFoundation = args?.foundation
if (resumedFoundation) {
  log(`Reusing verified foundation revision ${resumedFoundation.revision} after coordinator recovery.`)
}
const foundation = resumedFoundation ?? await agent(
  `${commonAssignment('foundation')}

The repository has no product commit. Create the executable foundation described by your role, including the existing Fleet design files and self-testing verifier. This is the only unit allowed to establish HEAD. Run verifier selftests and prove the complete known-good fixture plus one targeted known-bad mutation for each gate. Exact missing-root CLI exit behavior is outside acceptance. Do not implement fake feature stubs that claim acceptance. Return done=false for any missing foundation criterion.`,
  {
    agentType: 'cutokyo-architect',
    label: 'foundation',
    phase: 'Foundation',
    schema: BUILD_SCHEMA,
  },
)

if (!foundation || !foundation.done || !foundation.commits.length) {
  return { accepted: false, halted: 'foundation did not establish a verified committed base', foundation }
}

phase('Core')
const resumedCore = args?.core
if (resumedCore) {
  log(`Reusing recovered core revision ${resumedCore.revision} after coordinator recovery.`)
}
const core = resumedCore ?? await agent(
  `${commonAssignment('core')}

Build the durable core from the committed foundation. The minimum handoff is a real fake-observation -> atomic spool -> idempotent ingest -> SQLite/FTS -> app search pipeline, plus all storage, migration, concurrency, backup, health, and provenance criteria in your role. Report your generated worktree branch exactly; the architect will merge it.`,
  {
    agentType: 'cutokyo-core-builder',
    label: 'build:core',
    phase: 'Core',
    isolation: 'worktree',
    schema: BUILD_SCHEMA,
  },
)

if (!core || !core.done || !core.branch) {
  return { accepted: false, halted: 'durable core did not reach merge readiness', foundation, core }
}

phase('Core merge')
const coreMerge = await agent(
  `${commonAssignment('core-merge')}

Merge the durable core branch into the primary branch before feature fan-out.
Core branch: ${core.branch}
Core revision: ${core.revision}
Core handoff: ${JSON.stringify(core, null, 2)}

Resolve only mechanical foundation/core collisions. Run the real pipeline, architecture, migration, and static checks on the merged revision. If a core contract defect exists, route it and return done=false rather than hiding it.`,
  {
    agentType: 'cutokyo-architect',
    label: 'integrate:core',
    phase: 'Core merge',
    schema: INTEGRATION_SCHEMA,
  },
)

if (!coreMerge || !coreMerge.done) {
  return { accepted: false, halted: 'core integration failed', foundation, core, coreMerge }
}

phase('Feature build')
const resumedFeatures = args?.features
if (resumedFeatures) {
  log('Reusing six recovered feature handoffs after coordinator recovery.')
} else {
  log('Building harness adapters, extension plane, CLI/operations, and desktop from the same integrated core.')
}
const features = resumedFeatures ?? await parallel([
  () => agent(
    `${commonAssignment('harness-claude')}

Implement Claude Code support. Use official hooks, OTel, CLI, and local state only where each is documented. Test duplicate SessionStart delivery, resume session-ID drift, changing internal JSONL, subagent transcripts, disabled persistence, and exact native resume. Run setup only in a disposable HOME; perform a read-only dry run against installed Claude Code ${args?.claudeVersion ?? 'when available'}.`,
    { agentType: 'cutokyo-harness-builder', label: 'build:claude', phase: 'Feature build', isolation: 'worktree', schema: BUILD_SCHEMA },
  ),
  () => agent(
    `${commonAssignment('harness-codex')}

Implement Codex support. Prefer app-server/local APIs and documented OTel before state files. Preserve thread.id as resume target and treat thread.sessionId as the session-tree identity; never derive one from the other. Test paginated history, oversized resume responses, dropped tool results, incomplete resumed views, and exact CLI/app-server resume. Run setup only in a disposable HOME; perform a read-only dry run against installed Codex ${args?.codexVersion ?? 'when available'}.`,
    { agentType: 'cutokyo-harness-builder', label: 'build:codex', phase: 'Feature build', isolation: 'worktree', schema: BUILD_SCHEMA },
  ),
  () => agent(
    `${commonAssignment('harness-opencode')}

Implement OpenCode support. Prefer its documented plugin and server/SDK APIs. Test async lifecycle/finalization timing, server restart/port drift, session event subscriptions, V1/V2 plugin shape drift, unknown versions, and exact native resume. Run setup only in a disposable HOME; perform a read-only dry run against installed OpenCode ${args?.opencodeVersion ?? 'when available'}.`,
    { agentType: 'cutokyo-harness-builder', label: 'build:opencode', phase: 'Feature build', isolation: 'worktree', schema: BUILD_SCHEMA },
  ),
  () => agent(
    `${commonAssignment('extensions')}

Implement the plugin contract/verifier/examples, read-only Cutokyo MCP, central MCP broker, secret guards, explicit proxy fallback, and consented AI analysis against the integrated core interfaces. Use fake MCP/provider endpoints for deterministic acceptance.`,
    { agentType: 'cutokyo-extension-builder', label: 'build:extensions', phase: 'Feature build', isolation: 'worktree', schema: BUILD_SCHEMA },
  ),
  () => agent(
    `${commonAssignment('cli-ops')}

Implement the native CLI and operational/release surface. The npm package wraps cargo-dist output. All release operations in this run are dry-run or locally packed; do not publish or create a release.`,
    { agentType: 'cutokyo-cli-ops-builder', label: 'build:cli-ops', phase: 'Feature build', isolation: 'worktree', schema: BUILD_SCHEMA },
  ),
  () => agent(
    `${commonAssignment('desktop')}

Implement the complete Tauri/React product against app-command contracts and deterministic fixtures. Capture early screenshots so one visual defect cannot spread. Browser E2E is required, and actual Tauri/WebdriverIO coverage must be present or reported as a blocking gap.`,
    { agentType: 'cutokyo-desktop-builder', label: 'build:desktop', phase: 'Feature build', isolation: 'worktree', schema: BUILD_SCHEMA },
  ),
]).then((items) => items.filter(Boolean))

if (features.length !== 6 || features.some((item) => !item.done || !item.branch)) {
  return { accepted: false, halted: 'one or more feature units did not reach merge readiness', foundation, core, coreMerge, features }
}

phase('Integration')
let integration = await agent(
  `${commonAssignment('integration')}

Merge these six feature branches into the primary branch in this dependency order: harness adapters, extensions, CLI/ops, desktop. Preserve every unit's behavior and resolve only mechanical collisions.

Feature handoffs:
${JSON.stringify(features, null, 2)}

Run the complete integrated suite after merging. Pay special attention to app ports, shared Cargo features, schema/fixture registries, CLI/MCP names, Tauri command bindings, setup ownership, and release metadata. Return done=false if any branch is dropped, any required check cannot run, or a product decision remains unresolved.`,
  {
    agentType: 'cutokyo-architect',
    label: 'integrate:features',
    phase: 'Integration',
    schema: INTEGRATION_SCHEMA,
  },
)

if (!integration || !integration.done) {
  return { accepted: false, halted: 'feature integration failed', foundation, core, coreMerge, features, integration }
}

let accepted = false
let round = 0
let gates = null
let qa = null
let jev = null
let review = null
let report = null
const history = []
const blockerAttempts = {}

while (!accepted && round < 12) {
  round += 1
  phase('Verification')
  const verified = await parallel([
    () => agent(
      `${commonAssignment('gatekeeper')}

Verification round ${round}. Integrated revision: ${integration.revision}.
Run verifier selftests first, then every deterministic blocking/warning command in your role. Save logs under ${FLEET}/evidence/round-${round}/commands/. Do not install or repair anything.`,
      { agentType: 'cutokyo-gatekeeper', label: `gates:r${round}`, phase: 'Verification', schema: CHECK_SCHEMA },
    ),
    () => agent(
      `${commonAssignment('product-qa')}

Verification round ${round}. Integrated revision: ${integration.revision}.
Use the actual CLI and packaged/native Tauri app against fake worlds and safe installed-harness probes. Save screenshots and journey logs under ${FLEET}/evidence/round-${round}/qa/. Judge visual design as well as behavior. A browser-only app claim is a failure or explicit gap. Return exactly one journey for each required key, each with direct evidence: ${REQUIRED_QA_JOURNEYS.join(', ')}.`,
      { agentType: 'cutokyo-product-qa', label: `qa:r${round}`, phase: 'Verification', schema: QA_SCHEMA },
    ),
    () => agent(
      `${commonAssignment('jev-checker')}

Verification round ${round}. Integrated revision: ${integration.revision}.
Load and follow the installed jev-qa skill at /home/lucas/.codex/skills/jev-qa/SKILL.md. Start the integrated browser fixture mode with isolated Cutokyo config/data at the batch's 127.0.0.1:4173 URL. Validate ${FLEET}/jev-cases.yaml, then run that exact batch unchanged once and save its report under ${FLEET}/evidence/round-${round}/jev/. It defines exactly one independent case for each required key: ${REQUIRED_JEV_CASES.join(', ')}. Read every resulting screenshot into context, including PASS, ERROR, and BLOCKED outcomes. JEV is supplemental browser evidence and must not claim native Tauri or installed-harness coverage. Return ERROR/BLOCKED as fail with routable findings; never repair the product or lower an expectation.`,
      { agentType: 'cutokyo-jev-checker', label: `jev:haiku:r${round}`, phase: 'Verification', schema: JEV_SCHEMA },
    ),
  ])
  gates = verified[0]
  qa = verified[1]
  jev = verified[2]

  if (!gates || gates.selftests_ok === false) {
    return {
      accepted: false,
      halted: 'verifier selftest failed; downstream verdicts are void',
      round,
      foundation,
      core,
      features,
      integration,
      gates,
      qa,
      jev,
    }
  }

  phase('Review')
  review = await agent(
    `${commonAssignment('technical-review')}

Review round ${round}. Integrated revision: ${integration.revision}.
Judge every criterion C01-C20 in ${FLEET}/review.json by inspecting source, tests, built artifacts, schemas, command evidence, and QA evidence yourself.

Gate results (context, not proof):
${JSON.stringify(gates, null, 2)}

QA results (context, not proof):
${JSON.stringify(qa, null, 2)}

Haiku JEV browser results (supplemental context, not native proof):
${JSON.stringify(jev, null, 2)}

A criterion without an artifact citation is undetermined. Treat installed-harness gaps, native-app gaps, and package dry-run gaps honestly. Route every failure to an owning unit.`,
    { agentType: 'cutokyo-technical-reviewer', label: `review:r${round}`, phase: 'Review', schema: REVIEW_SCHEMA },
  )

  const gateFailures = (gates.results || [])
    .filter((result) => result.status === 'fail' || result.status === 'void')
    .map((result) => ({
      key: result.key,
      owner: result.owner,
      kind: result.status === 'void' ? 'dependency' : 'product',
      defect: result.diagnostic,
      location: result.evidence,
      reproduction: result.command,
    }))
  const failureCandidates = [
    ...gateFailures,
    ...(qa?.failures || []).map((failure) => ({
      key: failure.key,
      owner: failure.owner,
      kind: 'product',
      defect: failure.defect,
      location: failure.evidence,
      reproduction: failure.reproduction,
    })),
    ...(jev?.failures || []).map((failure) => ({
      key: failure.key,
      owner: failure.owner,
      kind: 'product',
      defect: failure.defect,
      location: failure.evidence,
      reproduction: failure.reproduction,
    })),
    ...(review?.failures || []),
  ]
  const failures = []
  for (const failure of failureCandidates) {
    const prior = failures.find((item) => item.key === failure.key)
    if (!prior) {
      failures.push(failure)
      continue
    }
    prior.owner = prior.owner === failure.owner ? prior.owner : 'integration'
    prior.kind = prior.kind === failure.kind ? prior.kind : 'product'
    prior.defect = `${prior.defect}\nAlso observed: ${failure.defect}`
    prior.location = `${prior.location}; ${failure.location}`
    prior.reproduction = `${prior.reproduction}\n${failure.reproduction}`
  }

  const gateComplete = gates.all_passed === true &&
    gates.revision === integration.revision &&
    gates.results.length > 0 &&
    gates.results.every((result) => result.status !== 'fail' && result.status !== 'void')
  const qaByKey = new Map((qa?.journeys || []).map((journey) => [journey.key, journey]))
  const qaComplete = qa?.verdict === 'pass' &&
    qa.revision === integration.revision &&
    qa.native_app_tested === true &&
    qa.installed_harnesses.length >= 3 &&
    qa.journeys.length === REQUIRED_QA_JOURNEYS.length &&
    REQUIRED_QA_JOURNEYS.every((key) => {
      const journey = qaByKey.get(key)
      return journey?.status === 'pass' && journey.evidence.length > 0
    })
  const jevByKey = new Map((jev?.cases || []).map((item) => [item.key, item]))
  const jevComplete = jev?.verdict === 'pass' &&
    jev.revision === integration.revision &&
    jev.validated === true &&
    jev.screenshots_read === true &&
    jev.batch.trim().length > 0 &&
    jev.report.trim().length > 0 &&
    jev.cases.length === REQUIRED_JEV_CASES.length &&
    REQUIRED_JEV_CASES.every((key) => {
      const item = jevByKey.get(key)
      return item?.status === 'PASS' && item.evidence.length > 0 && item.screenshots.length > 0
    })
  const reviewById = new Map((review?.criteria || []).map((criterion) => [criterion.id, criterion]))
  const reviewComplete = review?.verdict === 'pass' &&
    review.revision === integration.revision &&
    review.criteria.length === REQUIRED_CRITERIA.length &&
    REQUIRED_CRITERIA.every((id) => {
      const criterion = reviewById.get(id)
      return criterion?.verdict === 'pass' && criterion.citation.trim().length > 0
    })

  if (gates.all_passed === true && !gateComplete) {
    failures.push({
      key: 'gate-evidence-contract',
      owner: 'integration',
      kind: 'check',
      defect: 'Gatekeeper claimed pass with stale, empty, failed, or void evidence.',
      location: `${FLEET}/evidence/round-${round}/commands`,
      reproduction: 'Compare the gate revision and every structured result with integrated HEAD.',
    })
  }
  if (qa?.verdict === 'pass' && !qaComplete) {
    failures.push({
      key: 'qa-evidence-contract',
      owner: 'integration',
      kind: 'check',
      defect: 'Product QA claimed pass without same-revision native, installed-harness, or complete journey evidence.',
      location: `${FLEET}/evidence/round-${round}/qa`,
      reproduction: `Require exactly these passing journey keys with evidence: ${REQUIRED_QA_JOURNEYS.join(', ')}.`,
    })
  }
  if (jev?.verdict === 'pass' && !jevComplete) {
    failures.push({
      key: 'jev-evidence-contract',
      owner: 'integration',
      kind: 'check',
      defect: 'Haiku JEV claimed pass without a validated same-revision batch, exact case coverage, or inspected screenshot evidence.',
      location: `${FLEET}/evidence/round-${round}/jev`,
      reproduction: `Require exactly these PASS case keys with evidence and read screenshots: ${REQUIRED_JEV_CASES.join(', ')}.`,
    })
  }
  if (review?.verdict === 'pass' && !reviewComplete) {
    failures.push({
      key: 'review-evidence-contract',
      owner: 'integration',
      kind: 'check',
      defect: 'Technical review claimed pass without a same-revision cited pass for every C01-C20 criterion.',
      location: `${FLEET}/evidence/round-${round}/review`,
      reproduction: 'Require exactly one cited passing record for every criterion in review.json.',
    })
  }
  if (!failures.length && (!gateComplete || !qaComplete || !jevComplete || !reviewComplete)) {
    failures.push({
      key: 'unexplained-verdict',
      owner: 'integration',
      kind: 'check',
      defect: 'A verifier returned a non-passing or incomplete verdict without a routable finding.',
      location: `${FLEET}/evidence/round-${round}`,
      reproduction: 'Inspect structured gate, QA, and technical-review outputs.',
    })
  }

  accepted = gateComplete && qaComplete && jevComplete && reviewComplete
  history.push({
    round,
    revision: integration.revision,
    gates: gateComplete,
    qa: qaComplete,
    jev: jevComplete,
    review: reviewComplete,
    failures,
  })
  log(`round ${round}: gates=${gateComplete} qa=${qaComplete} jev=${jevComplete} review=${reviewComplete} findings=${failures.length}`)

  if (accepted) break

  const exhausted = failures
    .filter((failure) => (blockerAttempts[failure.key] || 0) >= 3)
    .map((failure) => [failure.key, blockerAttempts[failure.key]])
  if (exhausted.length) {
    return { accepted: false, halted: 'same blocker survived three recovery attempts', exhausted, history, gates, qa, jev, review }
  }
  for (const failure of failures) {
    blockerAttempts[failure.key] = (blockerAttempts[failure.key] || 0) + 1
  }

  phase('Repair')
  const triage = await agent(
    `${commonAssignment('repair-triage')}

Round ${round} failed. Inspect each finding and classify product defect, faulty check, conflicting requirement, or missing dependency. Do not edit, weaken, replace, or auto-repair a faulty check: put its key in unresolved, explain the exact contradiction in notes, and force this workflow to stop so the human can decide. Reassign every genuine product defect to one concrete builder owner; leave integration only for shared wiring or evidence-orchestration work, never as a route around a suspect check. Return every finding key exactly once: in assignments when product work remains, in resolved only when existing evidence already disproves the finding without changing a check, or in unresolved when human judgment/dependency resolution is required.

Findings:
${JSON.stringify(failures, null, 2)}`,
    { agentType: 'cutokyo-architect', label: `triage:r${round}`, phase: 'Repair', schema: TRIAGE_SCHEMA },
  )

  if (!triage) {
    return { accepted: false, halted: 'repair triage did not return a usable result', round, history, failures }
  }
  if (triage.invalid_check || triage.unresolved.length) {
    return { accepted: false, needsHuman: true, halted: 'human decision required for an invalid check, conflict, or missing dependency', round, triage, history, failures }
  }
  const routeCounts = {}
  for (const key of [...triage.assignments.map((item) => item.key), ...triage.resolved]) {
    routeCounts[key] = (routeCounts[key] || 0) + 1
  }
  const findingKeys = new Set(failures.map((failure) => failure.key))
  const untriaged = [...findingKeys].filter((key) => routeCounts[key] !== 1)
  const invented = Object.keys(routeCounts).filter((key) => !findingKeys.has(key))
  const invalidOwners = triage.assignments.filter((assignment) => !roleAssignments[assignment.owner])
  if (untriaged.length || invented.length || invalidOwners.length) {
    return {
      accepted: false,
      halted: 'repair triage omitted, duplicated, invented, or misrouted finding keys',
      round,
      triage,
      untriaged,
      invented,
      invalidOwners,
      history,
    }
  }

  const byOwner = {}
  for (const assignment of triage.assignments) {
    if (!byOwner[assignment.owner]) byOwner[assignment.owner] = []
    const original = failures.find((failure) => failure.key === assignment.key)
    byOwner[assignment.owner].push({ ...original, triage: assignment.diagnosis })
  }

  const repairOwners = Object.keys(byOwner)
  const repairs = await parallel(repairOwners.map((owner) => () => {
    const route = roleAssignments[owner]
    return agent(
      `${commonAssignment(owner)}

REPAIR after round ${round}. ${route.prompt}
Fix the product, not the check, in a fresh isolated worktree from current integrated HEAD. Reproduce every assigned finding first, implement the smallest complete fix, rerun owner and affected aggregate checks, and commit atomically. If the triage is wrong or a dependency is unavailable, return done=false with the evidence instead of forcing a green result.

Assigned findings:
${JSON.stringify(byOwner[owner], null, 2)}`,
      { agentType: route.agentType, label: `repair:${owner}:r${round}`, phase: 'Repair', isolation: 'worktree', schema: BUILD_SCHEMA },
    )
  })).then((items) => items.filter(Boolean))

  if (repairs.some((item) => !item.done || !item.branch)) {
    return { accepted: false, halted: 'a repair unit could not reach merge readiness', round, triage, repairs, history }
  }

  integration = await agent(
    `${commonAssignment('repair-integration')}

Merge the round-${round} repair branches into the primary branch and fix only merge artifacts.
Repair handoffs:
${JSON.stringify(repairs, null, 2)}

Unresolved integration/check/dependency findings from triage:
${JSON.stringify(triage?.unresolved || [], null, 2)}

Rerun every directly affected command plus the integrated smoke suite. Return done=false if an unresolved item prevents meaningful re-verification.`,
    { agentType: 'cutokyo-architect', label: `integrate:repair:r${round}`, phase: 'Repair', schema: INTEGRATION_SCHEMA },
  )

  if (!integration || !integration.done) {
    return { accepted: false, halted: 'repair integration failed', round, triage, repairs, integration, history }
  }
}

if (!accepted) {
  return {
    accepted: false,
    halted: 'global recovery safety limit reached before acceptance',
    rounds: round,
    revision: integration?.revision,
    history,
    gates,
    qa,
    jev,
    review,
  }
}

if (accepted) {
  phase('Report')
  report = await agent(
    `${commonAssignment('final-report')}

The integrated revision ${integration.revision} passed round ${round}. Build the final evidence report inside ${FLEET}, update review.json statuses/evidence without inventing anything, and render it with the installed fleet-review CLI. Include only concise final LLM verdict summaries, exact command results, genuine installed-harness coverage, package/native-app limitations, and screenshots of working journeys. Open the rendered local review in the browser. This is evidence for the already accepted revision: do not edit product/tests/gates or create a new commit, and report ${integration.revision} as the reviewed revision. Do not publish, push, or release.

History:
${JSON.stringify(history, null, 2)}

Final gates:
${JSON.stringify(gates, null, 2)}

Final QA:
${JSON.stringify(qa, null, 2)}

Final Haiku JEV browser checks:
${JSON.stringify(jev, null, 2)}

Final technical review:
${JSON.stringify(review, null, 2)}`,
    { agentType: 'cutokyo-architect', label: 'report', phase: 'Report', schema: REPORT_SCHEMA },
  )
}

const completed = accepted &&
  report?.revision === integration.revision &&
  report.rendered === true &&
  report.opened === true &&
  report.report.trim().length > 0 &&
  report.review.trim().length > 0

return {
  accepted: completed,
  productAccepted: accepted,
  halted: completed ? null : 'accepted product could not produce and open the same-revision final evidence review',
  rounds: round,
  revision: integration?.revision,
  foundation,
  core,
  features,
  integration,
  gates,
  qa,
  jev,
  review,
  history,
  report,
}

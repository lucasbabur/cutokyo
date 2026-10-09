export const meta = {
  name: 'cutokyo-finish',
  description: 'Finish Cutokyo with reviewed commit pins and complete same-revision evidence',
  whenToUse: 'After approval, supply a unique runKey to finish the C16, C17 and C19 candidates.',
  phases: [
    { title: 'Preflight', detail: 'pin the clean integration base without editing it' },
    { title: 'Candidates', detail: 'parallel builds, serial independent acceptance reviews' },
    { title: 'Integration', detail: 'merge only approved commit IDs' },
    { title: 'Verification', detail: 'serial gates, product QA, and Haiku JEV on port 4173' },
    { title: 'Review', detail: 'independent cited judgment of C01-C20' },
    { title: 'Repair', detail: 'bounded owner repairs with independent review before merging' },
    { title: 'Report', detail: 'preserve successful or unsuccessful evidence and verify the final receipt' },
  ],
}

const ROOT = '/home/lucas/Developer/personal/cutokyo-community'
const FLEET = `${ROOT}/.claude/fleets/20260919-cutokyo-v01`
const ACCEPT = `python3 ${ROOT}/tools/fleet/cutokyo-acceptance.py`
const GATES = `python3 ${ROOT}/tools/fleet/cutokyo-gates.py`
const DIFFGUARD = 'python3 /home/lucas/.claude/fleet-tools/diffguard/diffguard.py'
const CONTRACT_BASE = '1db95d8659d31df8cea6933a994d9a196205bf8e'
const MAX_CANDIDATE_ROUNDS = 3
const MAX_VERIFICATION_ROUNDS = 4 // initial verification plus at most three repair attempts
const REQUIRED_CRITERIA = Array.from({ length: 20 }, (_, i) => `C${String(i + 1).padStart(2, '0')}`)
const REQUIRED_HARNESSES = ['claude', 'codex', 'opencode']
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

// Workflow scripts have no filesystem APIs. Agents collect evidence; these predicates
// enforce the report contract. The last independent agent reads the physical artifacts.
const TEXT = { type: 'string' }
const BOOL = { type: 'boolean' }
const STRINGS = { type: 'array', items: TEXT }
const COMMANDS = { type: 'array', items: {
  type: 'object', properties: { command: TEXT, exit_code: { type: 'integer' }, evidence: TEXT },
  required: ['command', 'exit_code', 'evidence'],
} }
const FINDINGS = { type: 'array', items: {
  type: 'object', properties: {
    key: TEXT, owner: TEXT, defect: TEXT, location: TEXT, reproduction: TEXT, blocking: BOOL,
  },
  required: ['key', 'owner', 'defect', 'location', 'reproduction', 'blocking'],
} }
const PIN_SCHEMA = {
  type: 'object', properties: { revision: TEXT, clean: BOOL, contract_unchanged: BOOL, evidence: STRINGS, gaps: STRINGS },
  required: ['revision', 'clean', 'contract_unchanged', 'evidence', 'gaps'],
}
const BUILD_SCHEMA = {
  type: 'object', properties: {
    key: TEXT, done: BOOL, branch: TEXT, base_revision: TEXT, revision: TEXT, worktree: TEXT,
    commits: STRINGS, files_changed: STRINGS, commands: COMMANDS, mutations: STRINGS,
    evidence: STRINGS, gaps: STRINGS, notes: TEXT,
  },
  required: ['key', 'done', 'branch', 'base_revision', 'revision', 'worktree', 'commits', 'files_changed', 'commands', 'mutations', 'evidence', 'gaps', 'notes'],
}
const REVIEW_SCHEMA = {
  type: 'object', properties: {
    key: TEXT, verdict: { type: 'string', enum: ['pass', 'fail'] }, revision: TEXT,
    contract_unchanged: BOOL, ran_acceptance: BOOL, findings: FINDINGS,
    personally_run: COMMANDS, criteria_run: STRINGS, gaps: STRINGS, summary: TEXT,
  },
  required: ['key', 'verdict', 'revision', 'contract_unchanged', 'ran_acceptance', 'findings', 'personally_run', 'criteria_run', 'gaps', 'summary'],
}
const MERGE_PIN_SCHEMA = {
  type: 'object', properties: {
    revision: TEXT, clean: BOOL, contract_unchanged: BOOL, gaps: STRINGS,
    units: { type: 'array', items: {
      type: 'object', properties: { key: TEXT, branch: TEXT, revision: TEXT, worktree: TEXT, clean: BOOL, acceptance_verified: BOOL, evidence: TEXT },
      required: ['key', 'branch', 'revision', 'worktree', 'clean', 'acceptance_verified', 'evidence'],
    } },
  },
  required: ['revision', 'clean', 'contract_unchanged', 'gaps', 'units'],
}
const INTEGRATION_SCHEMA = {
  type: 'object', properties: {
    done: BOOL, base_revision: TEXT, revision: TEXT, merged_branches: STRINGS,
    merged_revisions: STRINGS, conflicts: STRINGS, commands: COMMANDS, gaps: STRINGS,
  },
  required: ['done', 'base_revision', 'revision', 'merged_branches', 'merged_revisions', 'conflicts', 'commands', 'gaps'],
}
const CHECK_SCHEMA = {
  type: 'object', properties: {
    selftests_ok: BOOL, all_passed: BOOL, revision: TEXT, commands: COMMANDS, gaps: STRINGS,
    results: { type: 'array', items: {
      type: 'object', properties: {
        key: TEXT, status: { type: 'string', enum: ['pass', 'fail', 'void', 'warn'] },
        owner: TEXT, command: TEXT, exit_code: { type: 'integer' }, diagnostic: TEXT, evidence: TEXT,
      },
      required: ['key', 'status', 'owner', 'command', 'exit_code', 'diagnostic', 'evidence'],
    } },
  },
  required: ['selftests_ok', 'all_passed', 'revision', 'commands', 'gaps', 'results'],
}
const QA_SCHEMA = {
  type: 'object', properties: {
    verdict: { type: 'string', enum: ['pass', 'fail'] }, revision: TEXT, native_app_tested: BOOL,
    installed_harnesses: STRINGS, screenshots: STRINGS,
    journeys: { type: 'array', items: {
      type: 'object', properties: {
        key: TEXT, status: { type: 'string', enum: ['pass', 'fail', 'blocked'] },
        evidence: STRINGS, action_logs: STRINGS, summary: TEXT,
      },
      required: ['key', 'status', 'evidence', 'action_logs', 'summary'],
    } },
    failures: FINDINGS, gaps: STRINGS, summary: TEXT,
  },
  required: ['verdict', 'revision', 'native_app_tested', 'installed_harnesses', 'screenshots', 'journeys', 'failures', 'gaps', 'summary'],
}
const JEV_SCHEMA = {
  type: 'object', properties: {
    verdict: { type: 'string', enum: ['pass', 'fail'] }, revision: TEXT,
    validated: BOOL, screenshots_read: BOOL, batch: TEXT, report: TEXT,
    cases: { type: 'array', items: {
      type: 'object', properties: {
        key: TEXT, status: { type: 'string', enum: ['PASS', 'FAIL', 'ERROR', 'BLOCKED'] },
        evidence: STRINGS, action_logs: STRINGS, screenshots: STRINGS, summary: TEXT,
      },
      required: ['key', 'status', 'evidence', 'action_logs', 'screenshots', 'summary'],
    } },
    failures: FINDINGS, gaps: STRINGS, summary: TEXT,
  },
  required: ['verdict', 'revision', 'validated', 'screenshots_read', 'batch', 'report', 'cases', 'failures', 'gaps', 'summary'],
}
const FINAL_REVIEW_SCHEMA = {
  type: 'object', properties: {
    verdict: { type: 'string', enum: ['pass', 'fail'] }, revision: TEXT,
    criteria: { type: 'array', items: {
      type: 'object', properties: { id: TEXT, verdict: { type: 'string', enum: ['pass', 'fail'] }, citation: TEXT, basis: TEXT },
      required: ['id', 'verdict', 'citation', 'basis'],
    } },
    findings: FINDINGS, gaps: STRINGS, summary: TEXT,
  },
  required: ['verdict', 'revision', 'criteria', 'findings', 'gaps', 'summary'],
}
const REPORT_SCHEMA = {
  type: 'object', properties: { revision: TEXT, report_path: TEXT, html_path: TEXT, summary: TEXT, gaps: STRINGS },
  required: ['revision', 'report_path', 'html_path', 'summary', 'gaps'],
}
const RECEIPT_SCHEMA = {
  type: 'object', properties: {
    revision: TEXT, clean: BOOL, contract_unchanged: BOOL, evidence_matches_revision: BOOL,
    artifacts: { type: 'array', items: {
      type: 'object', properties: { path: TEXT, readable: BOOL, sha256: TEXT },
      required: ['path', 'readable', 'sha256'],
    } },
    findings: FINDINGS, gaps: STRINGS,
  },
  required: ['revision', 'clean', 'contract_unchanged', 'evidence_matches_revision', 'artifacts', 'findings', 'gaps'],
}

const OWNERS = {
  core: 'cutokyo-core-builder', 'cli-ops': 'cutokyo-cli-ops-builder',
  desktop: 'cutokyo-desktop-builder', extensions: 'cutokyo-extension-builder',
  'harness-claude': 'cutokyo-harness-builder', 'harness-codex': 'cutokyo-harness-builder',
  'harness-opencode': 'cutokyo-harness-builder', harness: 'cutokyo-harness-builder',
  integration: 'cutokyo-architect', architect: 'cutokyo-architect',
}
const CANDIDATES = [
  {
    key: 'C16', head: '66deba6e16bb22226681ed9f5dcb95d3b03dba00', owner: 'cli-ops', repaired: true,
    brief: 'Independently verify the existing diagnostics repair before changing it: metadata-only allowlisted logs, classified panic thread names, crash clearing bound to a published bundle receipt including replacement records, the real CLI binary in the exact named test, decoded-JSON Windows-path leak inspection, core/CLI/desktop health parity, and bounded health reads without growing-history scans.',
  },
  {
    key: 'C19', head: 'ff089c23ef4e651b84f94266f281dfe5b82156d6', owner: 'cli-ops',
    brief: 'Finish the unreviewed release repair. Verify Windows shell syntax; source snapshot detection of transient writes, evidence paths, replaced captures and moved HEAD/index; real updater signature verification; npm environment isolation and enforced egress denial; actual installed Linux package startup/uninstall; the cargo-dist archive in npm version/doctor smoke; substantive CycloneDX/provenance and workflow validation; complete checksums; and correct macOS architecture. Establish what ff089c2 already fixes before editing.',
  },
  {
    key: 'C17', head: '85e9229720f166f6600afbd43ff890a5b03dfb3f', owner: 'desktop',
    brief: 'Repair native desktop findings: launch the installed/bundled Linux package rather than target/debug; isolate native E2E assets from production ui/dist; isolate concurrent ports and roots and kill the app before cleanup; assert actual 1280x800 geometry; publish fake-resume audit contents atomically; gate fixture seeding on native-e2e plus validated test mode. Use the local cutokyo-tauri-qa:2.11.4 Docker image with Xvfb and D-Bus; missing host WebKit/DBus packages are environment gaps, not invalid requirements.',
  },
]

const text = (value) => typeof value === 'string' && value.trim().length > 0
const sha = (value) => typeof value === 'string' && /^[0-9a-f]{40}$/.test(value)
const empty = (value) => Array.isArray(value) && value.length === 0
const paths = (value) => Array.isArray(value) && value.length > 0 && value.every((p) => text(p) && p.startsWith('/'))
function exact(values, expected) {
  return Array.isArray(values) && values.length === expected.length &&
    new Set(values).size === expected.length && expected.every((key) => values.includes(key))
}
function rows(items, field, expected) {
  return Array.isArray(items) && items.every((item) => item && typeof item === 'object') &&
    exact(items.map((item) => item[field]), expected)
}
function commandsOk(commands) {
  return Array.isArray(commands) && commands.length > 0 && commands.every((c) =>
    c && text(c.command) && c.exit_code === 0 && paths([c.evidence]))
}
function staticCommands(base) {
  return [`${GATES} selftest`, `${GATES} all --root ${ROOT}`, `${DIFFGUARD} selftest`, `${DIFFGUARD} scan --repo ${ROOT} --base ${base} --json`]
}
function staticOk(commands, base) {
  return commandsOk(commands) && staticCommands(base).every((command) => commands.some((c) => c.command === command))
}
function human(value) {
  return /invalid-check-human/i.test(JSON.stringify(value) || '')
}

// A caller-supplied namespace is stable on resume and avoids timestamps/randomness.
if (typeof args?.runKey !== 'string' || !/^[a-z0-9][a-z0-9-]{0,47}$/.test(args.runKey)) {
  return { accepted: false, halted: 'supply a unique lowercase runKey (1-48 letters, digits or hyphens); reuse only to recover this run' }
}
const RUN = args.runKey
const OUT = `/home/lucas/Developer/personal/cutokyo-community-finish-evidence/${RUN}`
const WORKTREES = `/home/lucas/Developer/personal/cutokyo-community-finish-worktrees/${RUN}`
const COMMON = `You are working in Fleet 20260920-cutokyo-finish, run ${RUN}.
Repository: ${ROOT}
Read ${FLEET}/review.json and ${FLEET}/CONTEXT.md. Evidence root: ${OUT}/evidence.
Hard boundaries:
- review.json and its acceptance commands must remain BYTE IDENTICAL to ${CONTRACT_BASE}. No reformatting or status edits.
- Never weaken, skip, delete or work around a check. A genuinely invalid requirement means STOP and return an invalid-check-human gap with the exact contradiction and evidence. A missing dependency is an environment-gap, NOT automatically an invalid requirement. Exit 86 means insufficient successful test execution evidence, including missing/ignored-only/inconsistent test results: fix the tests, never change the filter.
- Never push, tag, publish or release. Never read or copy /home/lucas/Developer/personal/cutokyo.
- Never inspect credentials or read/copy/mutate real harness configuration. Use disposable HOME/config/data; installed-harness probes are credential-free version/help probes only.
- Do not load or bind the parent's Herdr session in any agent. No parent task/status writes.
- No fabricated evidence. Evidence/citation fields are absolute paths to actual nonempty artifacts, not summaries. Preserve command stdout/stderr using --log-dir, not just the runner's --json summary. For each acceptance command, evidence must name its <log-dir>/<criterion>-report.json receipt. Preserve and inspect all segment logs referenced in it. The receipt must be complete/pass, command_unmodified, on the claimed revision, with verified pinned contract identity and environment metadata. For EVERY live acceptance invocation, inspect/create its assigned criterion parent directory and use mktemp -d <that-parent>/attempt-XXXXXX to allocate a fresh directory. Replace the command template's attempt-XXXXXX with that actual resolved basename and report the resolved command, not shell substitution. Never execute the literal template. Never overwrite or delete old receipts/logs. A noncached retry under the same runKey gets a fresh attempt directory; a cached completed call may reuse its immutable evidence only after independent physical revalidation. Preserve unsuccessful artifacts. Never overwrite existing report files; the report assignment also uses a fresh mktemp directory.
- Verify full git HEAD and a clean tree before AND after any review/verification. Return the actual revision; never echo an expected revision if HEAD moved. Never mutate a reviewed tree.
- Browser and JEV observations do not substitute for native Tauri or installed-harness evidence.
- Port 4173 is shared and Playwright reuseExistingServer is false. Stop and wait for every server/process you started before returning; never kill an unrelated server. An occupied port is an environment-gap. Builders must not run browser/native acceptance in parallel; serial independent reviewers run it.
- Commit messages end with:
Signed-off-by: Lucasbabur <lucasbabur@gmail.com>
Co-Authored-By: Claude Code <noreply@anthropic.com>
- Only an explicit integration assignment may edit ${ROOT} on main. Builders use their dedicated durable worktrees. Report agents write only the assigned evidence files, never source or review.json.`

function workspace(key, base) {
  return { key, base, branch: `fix/cutokyo-finish-${RUN}-${key.toLowerCase()}`, worktree: `${WORKTREES}/${key.toLowerCase()}` }
}
function workspaceRules(unit) {
  return `Use ONLY this workspace: ${JSON.stringify(unit)}.
Inspect git worktree list --porcelain, the path (including symlinks), and refs before acting. Never overwrite, reset, force, delete, prune or remove an existing worktree or branch.
For an absent path AND absent branch, create the parent directory then run:
  git -C ${ROOT} worktree add -b ${unit.branch} ${unit.worktree} ${unit.base}
For an existing branch but absent path, first confirm the branch descends from ${unit.base} and is not checked out elsewhere; then git -C ${ROOT} worktree add ${unit.worktree} ${unit.branch}.
For an existing path, reject symlinks/non-worktrees; require git rev-parse --show-toplevel to equal ${unit.worktree}, git symbolic-ref --short HEAD to equal ${unit.branch}, and git status --porcelain to be empty. Require git merge-base --is-ancestor ${unit.base} HEAD. Otherwise STOP with a recovery gap, preserving everything. Reuse only this run's matching clean worktree. Never use main as the worktree's branch. Report base_revision=${unit.base}.`
}
function acceptance(key, root, label) {
  return `${ACCEPT} run --criterion ${key} --root ${root} --json --log-dir ${OUT}/evidence/${label}/${key}/attempt-XXXXXX`
}
function acceptanceMatches(record, key, root, label) {
  const prefix = acceptance(key, root, label).replace(/XXXXXX$/, '')
  if (!record.command.startsWith(prefix) || record.command.slice(prefix.length) === 'XXXXXX' ||
      !/^[A-Za-z0-9]{6,32}$/.test(record.command.slice(prefix.length))) return false
  const directory = record.command.slice(record.command.indexOf(' --log-dir ') + ' --log-dir '.length)
  return record.evidence === `${directory}/${key}-report.json`
}
function buildOk(build, unit) {
  return build?.key === unit.key && build.done === true && build.branch === unit.branch &&
    build.base_revision === unit.base && build.worktree === unit.worktree && sha(build.revision) &&
    Array.isArray(build.commits) && build.commits.includes(build.revision) && build.commits.every(sha) &&
    commandsOk(build.commands) && paths(build.evidence) && empty(build.gaps)
}
function reviewOk(review, build, criteria, label) {
  return review?.key === build.key && review.verdict === 'pass' && review.revision === build.revision &&
    review.contract_unchanged === true && review.ran_acceptance === true &&
    empty(review.findings) && empty(review.gaps) && commandsOk(review.personally_run) &&
    Array.isArray(review.criteria_run) && review.criteria_run.length > 0 &&
    new Set(review.criteria_run).size === review.criteria_run.length &&
    criteria.every((key) => review.criteria_run.includes(key)) &&
    review.criteria_run.every((key) => REQUIRED_CRITERIA.includes(key) &&
      review.personally_run.some((c) => acceptanceMatches(c, key, build.worktree, label)))
}
function reviewPrompt(unit, build, criteria, label, findings) {
  return `${COMMON}
Independently review ${unit.key} at full revision ${build.revision} in ${unit.worktree}; expected branch ${unit.branch}.
Verify the worktree's HEAD equals that pin, its base is ${unit.base}, and it is clean. Compare the acceptance contract byte-for-byte against ${CONTRACT_BASE}. Inspect every changed file and actual artifacts; the builder's handoff is context, not proof:
${JSON.stringify(build)}
Assigned criteria: ${criteria.join(', ')}. Findings to independently reproduce and verify cleared: ${JSON.stringify(findings)}.
Run ALL of these exact commands yourself, plus every other criterion affected by the changed files, recording their IDs in criteria_run and exit codes/raw-log evidence in personally_run:
${criteria.map((key) => acceptance(key, unit.worktree, label)).join('\n')}
For additional affected criteria use this exact command form with the criterion ID substituted:
${acceptance('<ID>', unit.worktree, label)}
criteria_run must be nonempty even when the finding is a QA/JEV journey without a criterion ID; independently map that journey to its acceptance criterion. Do not require an unrelated owner's still-failing criterion as proof of this repair; all twenty rerun after integration.
Check for tautological assertions, helper-only tests, production APIs widened for tests, privacy leaks, races and fabricated evidence. Inspect mutation evidence. Do not edit, repair or commit anything. Return pass only with contract_unchanged, ran_acceptance, no outstanding findings/gaps and personally collected evidence. A failure must have concrete file locations, reproduction steps and owner keys. Do not claim a pass with even nonblocking outstanding findings.`
}

// Preserve nulls and thrown-agent diagnostics. A parallel API may drop an item;
// the fixed expected key sets below must still match before any integration.
const calls = []
async function ask(prompt, options) {
  try {
    const result = await agent(prompt, options)
    calls.push({ label: options.label, result })
    return result
  } catch (error) {
    calls.push({ label: options.label, result: null, error: String(error) })
    return null
  }
}
const state = { candidates: [], integrations: [], history: [], repairs: [], calls }
function stopped(halted, evidence = {}) {
  return { accepted: false, halted, needsHuman: human(evidence) || human(calls), ...evidence }
}
async function integrate(units, base, label) {
  phase('Integration')
  const checked = await ask(`${COMMON}
READ-ONLY merge preflight, not permission to integrate. Require main HEAD=${base}, clean tree and unchanged contract. Independently resolve the actual branch AND worktree HEAD of each approved unit; both must equal its reviewed pin. Verify the contract on every input and preserve git output under ${OUT}/evidence/${label}/pins.
Approved inputs: ${JSON.stringify(units.map(({ build }) => ({ key: build.key, branch: build.branch, revision: build.revision, worktree: build.worktree })))}
Independent review receipts: ${JSON.stringify(units.map(({ review }) => review))}
Open each personally_run receipt and its referenced segment logs. Require complete/pass, exit_code=0, command_unmodified=true, contract.verified=true with the immutable contract identity, the reviewed revision, clean tracked state, unchanged source_after identity and actual successful test execution. Mark each unit acceptance_verified=true only after this physical inspection. Missing or contradictory evidence stops integration.
Return actual revision/branch/worktree/clean state and evidence for every unit; do not echo expected values when they differ. No merge, source edits or repairs.`,
  { agentType: 'cutokyo-technical-reviewer', label: `pin:${label}`, phase: 'Integration', schema: MERGE_PIN_SCHEMA })
  if (human(checked) || checked?.revision !== base || checked.clean !== true || checked.contract_unchanged !== true ||
      !empty(checked.gaps) || !rows(checked.units, 'key', units.map((u) => u.build.key)) ||
      !units.every(({ build }) => checked.units.some((u) => u.key === build.key && u.branch === build.branch &&
        u.revision === build.revision && u.worktree === build.worktree && u.clean === true && u.acceptance_verified === true && paths([u.evidence])))) return null
  return ask(`${COMMON}
Integrate ONLY the following independently approved commit pins into ${ROOT} on main. First require clean main HEAD == ${base}; if it moved, STOP without merging.
${JSON.stringify(units.map(({ build, review }) => ({ key: build.key, branch: build.branch, revision: build.revision, review })))}
Before ANY merge, verify EVERY listed branch/worktree still equals its reviewed full SHA and is clean, every pinned commit exists, and review.json is byte-identical to ${CONTRACT_BASE} on every input. A moved branch is a stop, never permission to merge its new head.
Merge only these immutable SHAs, NEVER a branch name:
${units.map(({ build }) => `git -C ${ROOT} merge --no-ff --no-commit ${build.revision}`).join('\n')}
After each merge, commit it with the required Signed-off-by and Co-Authored-By trailers before starting the next. Preserve all behavior and tests; stop on a nonmechanical conflict or dropped unit. Report the exact merged_revisions and merged_branches, base_revision=${base}, and final full revision. Require all pins are ancestors of the final HEAD. Do not amend a reviewed commit.
Run ${GATES} selftest, ${GATES} all --root ${ROOT}, ${DIFFGUARD} selftest and ${DIFFGUARD} scan --repo ${ROOT} --base ${base} --json. Capture each command's stdout/stderr and real exit code under ${OUT}/evidence/${label}. The next serial verification runs all twenty unchanged acceptance commands. Leave main clean. No required check or unit may be dropped. Return done=false on any failure or gap.`,
  { agentType: 'cutokyo-architect', label, phase: 'Integration', schema: INTEGRATION_SCHEMA })
}
function integrationOk(result, units, base) {
  return result?.done === true && result.base_revision === base && sha(result.revision) &&
    result.revision !== base && exact(result.merged_revisions, units.map((u) => u.build.revision)) &&
    exact(result.merged_branches, units.map((u) => u.build.branch)) &&
    empty(result.conflicts) && empty(result.gaps) && staticOk(result.commands, base)
}

async function run() {
  log(`Limits: ${MAX_CANDIDATE_ROUNDS} candidate build/review attempts; ${MAX_VERIFICATION_ROUNDS} verification rounds (at most three repair attempts). Null, missing or unapproved units stop integration. No automatic retry of malformed evidence.`)
  phase('Preflight')
  const pin = await ask(`${COMMON}
Read-only preflight. Confirm ${ROOT} is clean on main. Resolve full HEAD, compare review.json and every recorded acceptance command byte-for-byte to ${CONTRACT_BASE}, and save the git/contract checks under ${OUT}/evidence/preflight. Do not merge, repair or change source. Return actual revision, clean, contract_unchanged, evidence and gaps.`,
  { agentType: 'cutokyo-technical-reviewer', label: 'pin', phase: 'Preflight', schema: PIN_SCHEMA })
  state.pin = pin
  if (human(pin)) return stopped('human decision required before building', { pin })
  if (!sha(pin?.revision) || pin.clean !== true || pin.contract_unchanged !== true || !paths(pin.evidence) || !empty(pin.gaps)) {
    return stopped('preflight did not establish a clean, unchanged integration base', { pin })
  }
  state.revision = pin.revision
  phase('Candidates')
  const candidates = CANDIDATES.map((c) => ({ ...c, unit: workspace(c.key, c.head), findings: [], approved: false }))
  state.candidates = candidates
  for (let attempt = 1; attempt <= MAX_CANDIDATE_ROUNDS; attempt++) {
    const pending = candidates.filter((c) => !c.approved)
    const builds = await parallel(pending.map((candidate) => () => ask(`${COMMON}
Candidate ${candidate.key}, attempt ${attempt}/${MAX_CANDIDATE_ROUNDS}. ${candidate.brief}
${workspaceRules(candidate.unit)}
${candidate.repaired && attempt === 1 ? 'This candidate is already repaired: materialize/verify its pinned worktree without changing it, for immediate independent review.' : 'Repair only this criterion and the independently reported findings.'}
Findings: ${JSON.stringify(candidate.findings)}.
Read the full criterion. Mutation-prove load-bearing changes and restore mutations before committing. Run affected non-browser tests, fmt, warning-denying clippy, gate selftests and diffguard; keep raw logs. Do NOT run browser/native acceptance during parallel builds; the serial reviewer will personally run ${acceptance(candidate.key, candidate.unit.worktree, `candidate-${attempt}-${candidate.key}`)}.
Report done only with clean committed work, full revision and real evidence; disclose all gaps.`,
    { agentType: OWNERS[candidate.owner], label: `build:${candidate.key}:r${attempt}`, phase: 'Candidates', schema: BUILD_SCHEMA })))
    state.candidateAttempt = { attempt, builds }
    if (human(builds) || human(calls)) return stopped('human decision required by a candidate', { builds })
    if (!rows(builds, 'key', pending.map((c) => c.key))) return stopped('candidate build results dropped, duplicated or missing', { builds })
    for (const candidate of pending) {
      candidate.build = builds.find((b) => b.key === candidate.key)
      candidate.attempts = attempt
      if (!buildOk(candidate.build, candidate.unit)) return stopped('candidate build is not merge-ready', { candidate })
    }
    // Reviews run acceptance on the shared browser port, so they are serial.
    for (const candidate of pending) {
      const label = `candidate-${attempt}-${candidate.key}`
      candidate.review = await ask(reviewPrompt(candidate.unit, candidate.build, [candidate.key], label, candidate.findings),
        { agentType: 'cutokyo-technical-reviewer', label: `review:${candidate.key}:r${attempt}`, phase: 'Candidates', schema: REVIEW_SCHEMA })
      const review = candidate.review
      if (human(review)) return stopped('human decision required by candidate review', { candidate })
      candidate.approved = reviewOk(review, candidate.build, [candidate.key], label)
      if (candidate.approved) continue
      // A genuine, evidenced rejection may be repaired. Malformed/stale or
      // contradictory approvals may not be converted into another builder task.
      if (review?.verdict !== 'fail' || review.revision !== candidate.build.revision ||
          review.key !== candidate.key || review.contract_unchanged !== true ||
          review.ran_acceptance !== true || !empty(review.gaps) ||
          !Array.isArray(review.findings) || !review.findings.length) {
        return stopped('candidate review lacks a valid same-revision approval or routable rejection', { candidate })
      }
      candidate.findings = review.findings
      log(`${candidate.key} rejected on attempt ${attempt}; all findings retained.`)
    }
    if (candidates.every((c) => c.approved)) break
  }
  if (candidates.some((c) => !c.approved)) return stopped('candidate review limit reached; no candidates integrated', { candidates })

  let integration = await integrate(candidates, pin.revision, 'integrate')
  state.integrations.push(integration)
  if (human(integration)) return stopped('human decision required by integration', { integration })
  if (!integrationOk(integration, candidates, pin.revision)) return stopped('integration receipt incomplete or inconsistent', { integration })
  state.revision = integration.revision

  for (let round = 1; round <= MAX_VERIFICATION_ROUNDS; round++) {
    const revision = integration.revision
    const label = `round-${round}`
    const entry = { round, revision }
    state.history.push(entry)
    phase('Verification')
    // These three agents ALL own port 4173. Never parallelize them.
    entry.gates = await ask(`${COMMON}
Verification round ${round} at pinned integrated revision ${revision} in ${ROOT}.
Run ${GATES} selftest, ${GATES} all --root ${ROOT}, ${DIFFGUARD} selftest and ${DIFFGUARD} scan --repo ${ROOT} --base ${pin.revision} --json; record raw logs and exits in commands. Then run exactly one entry for every C01-C20:
${REQUIRED_CRITERIA.map((key) => acceptance(key, ROOT, `${label}/commands`)).join('\n')}
Each result needs key, pass/fail/void/warn, actual command/exit code and a nonempty raw-log artifact path. Exit 86 is failure. Missing dependency means void plus an environment-gap, never an invalid requirement. Run no repairs or installs. Stop all servers you started before returning.`,
    { agentType: 'cutokyo-gatekeeper', label: `gates:r${round}`, phase: 'Verification', schema: CHECK_SCHEMA })
    const gates = entry.gates
    if (human(gates)) return stopped('human decision required by gates', { entry })
    if (!gates || gates.revision !== revision || gates.selftests_ok !== true || !empty(gates.gaps) ||
        !staticOk(gates.commands, pin.revision) || !rows(gates.results, 'key', REQUIRED_CRITERIA) ||
        !gates.results.every((r) => text(r.command) && acceptanceMatches(r, r.key, ROOT, `${label}/commands`) &&
          ['pass', 'fail'].includes(r.status) && Number.isInteger(r.exit_code) && (r.status === 'pass') === (r.exit_code === 0)) ||
        gates.all_passed !== gates.results.every((r) => r.status === 'pass')) {
      return stopped('gate evidence incomplete, stale, contradictory or environment-blocked', { entry })
    }
    entry.qa = await ask(`${COMMON}
Product QA round ${round}, pinned revision ${revision} in ${ROOT}. Load cutokyo-runtime-qa. Use actual CLI and packaged native Tauri with disposable roots, fake worlds and safe installed version/help probes, never real configuration. Native environment: cutokyo-tauri-qa:2.11.4 with Xvfb and D-Bus.
Return exactly these 27 journey keys: ${REQUIRED_QA_JOURNEYS.join(', ')}.
Every journey needs nonempty evidence and action_logs paths. Include genuine native/browser screenshots in screenshots and identify the environment/revision in the logs. installed_harnesses must contain exactly claude, codex, opencode, only if personally probed. Exercise design, keyboard, honest absent-versus-zero and coverage copy. Preserve evidence under ${OUT}/evidence/${label}/qa. Native-app and installed-harness gaps block acceptance. No repairs. Stop your servers before returning.`,
    { agentType: 'cutokyo-product-qa', label: `qa:r${round}`, phase: 'Verification', schema: QA_SCHEMA })
    const qa = entry.qa
    if (human(qa)) return stopped('human decision required by QA', { entry })
    if (!qa || qa.revision !== revision || !rows(qa.journeys, 'key', REQUIRED_QA_JOURNEYS) ||
        !qa.journeys.every((j) => ['pass', 'fail'].includes(j.status) && paths(j.evidence) && paths(j.action_logs)) ||
        !paths(qa.screenshots) || !exact(qa.installed_harnesses, REQUIRED_HARNESSES) || qa.native_app_tested !== true ||
        !Array.isArray(qa.failures) || !empty(qa.gaps) ||
        !['pass', 'fail'].includes(qa.verdict) || (qa.verdict === 'pass' && (qa.journeys.some((j) => j.status !== 'pass') || !empty(qa.failures)))) {
      return stopped('QA evidence incomplete, stale, contradictory or environment-blocked', { entry })
    }
    entry.jev = await ask(`${COMMON}
Haiku JEV round ${round}, pinned revision ${revision} in ${ROOT}. Load /home/lucas/.codex/skills/jev-qa/SKILL.md. Validate ${FLEET}/jev-cases.yaml, run that exact batch unchanged once against isolated browser fixtures at 127.0.0.1:4173, save report/action logs/screenshots under ${OUT}/evidence/${label}/jev.
Exactly eight unique case keys: ${REQUIRED_JEV_CASES.join(', ')}. Each case requires evidence[], action_logs[] and screenshots[] with actual absolute paths. Read EVERY screenshot, including PASS/ERROR/BLOCKED. ERROR/BLOCKED fail acceptance and need routable failures or environment gaps. Browser evidence cannot replace native or installed-harness evidence. No repairs. Stop your servers before returning.`,
    { agentType: 'cutokyo-jev-checker', label: `jev:r${round}`, phase: 'Verification', schema: JEV_SCHEMA })
    const jev = entry.jev
    if (human(jev)) return stopped('human decision required by JEV', { entry })
    if (!jev || jev.revision !== revision || jev.validated !== true || jev.screenshots_read !== true ||
        jev.batch !== `${FLEET}/jev-cases.yaml` || !paths([jev.report]) || !rows(jev.cases, 'key', REQUIRED_JEV_CASES) ||
        !jev.cases.every((c) => ['PASS', 'FAIL', 'ERROR', 'BLOCKED'].includes(c.status) && paths(c.evidence) && paths(c.action_logs) && paths(c.screenshots)) ||
        !Array.isArray(jev.failures) || !empty(jev.gaps) || !['pass', 'fail'].includes(jev.verdict) ||
        (jev.verdict === 'pass' && (jev.cases.some((c) => c.status !== 'PASS') || !empty(jev.failures)))) {
      return stopped('JEV evidence incomplete, stale or contradictory', { entry })
    }
    phase('Review')
    entry.review = await ask(`${COMMON}
Final independent technical review, round ${round}, pinned revision ${revision} in ${ROOT}.
Judge every C01-C20 criterion by inspecting source, tests and actual artifacts, not builder summaries. Read the original contract, including privacy, clean-room, native/installed-harness and release boundaries. Check evidence against this exact clean HEAD. Independently run cheap load-bearing probes.
Reports are context only: ${JSON.stringify({ gates, qa, jev })}
Return exactly one criterion row for every C01-C20, each with a nonempty citation to an absolute artifact path and a basis explaining exact observations and source file:line locations. Outstanding findings, missing evidence and gaps forbid pass. Route product findings to owner keys; flag genuinely invalid requirements as invalid-check-human and missing dependencies as environment-gap. Never edit requirements or code.`,
    { agentType: 'cutokyo-technical-reviewer', label: `review:r${round}`, phase: 'Review', schema: FINAL_REVIEW_SCHEMA })
    const review = entry.review
    if (human(review)) return stopped('human decision required by final review', { entry })
    if (!review || review.revision !== revision || !rows(review.criteria, 'id', REQUIRED_CRITERIA) ||
        !review.criteria.every((c) => ['pass', 'fail'].includes(c.verdict) && paths([c.citation]) && text(c.basis)) ||
        !Array.isArray(review.findings) || !empty(review.gaps) || !['pass', 'fail'].includes(review.verdict) ||
        (review.verdict === 'pass' && (review.criteria.some((c) => c.verdict !== 'pass') || !empty(review.findings)))) {
      return stopped('final review incomplete, stale or contradictory', { entry })
    }
    const failures = [
      ...gates.results.filter((r) => r.status !== 'pass').map((r) => ({ key: r.key, owner: r.owner, defect: r.diagnostic, location: r.evidence, reproduction: r.command, blocking: true })),
      ...qa.failures, ...jev.failures, ...review.findings,
    ]
    entry.failures = failures
    const accepted = gates.all_passed === true && qa.verdict === 'pass' && jev.verdict === 'pass' && review.verdict === 'pass' && empty(failures)
    log(`round ${round}: accepted=${accepted}; outstanding findings=${failures.length}`)
    if (accepted) return { accepted: true, revision }
    // Every nonpassing journey/case/criterion must be accounted for, not just
    // some unrelated finding that happens to make failures nonempty.
    const missingFindings = [
      ...qa.journeys.filter((j) => j.status !== 'pass' && !qa.failures.some((f) => f.key === j.key)),
      ...jev.cases.filter((c) => c.status !== 'PASS' && !jev.failures.some((f) => f.key === c.key)),
      ...review.criteria.filter((c) => c.verdict !== 'pass' && !review.findings.some((f) => f.key === c.id)),
    ]
    if (!failures.length || missingFindings.length || failures.some((f) => !f || !OWNERS[f.owner] || !text(f.key) || !text(f.defect) || !text(f.location) || !text(f.reproduction))) {
      return stopped('nonpassing reports lack complete routable findings', { entry, missingFindings })
    }
    if (round === MAX_VERIFICATION_ROUNDS) return stopped('verification limit reached after three repair attempts', { entry })

    phase('Repair')
    const owners = [...new Set(failures.map((f) => f.owner))]
    const units = owners.map((owner) => ({
      owner, unit: workspace(`repair-${round}-${owner}`, revision),
      findings: failures.filter((f) => f.owner === owner),
    }))
    const repairRound = { round, base_revision: revision, units }
    state.repairs.push(repairRound)
    const builds = await parallel(units.map(({ owner, unit, findings }) => () => ask(`${COMMON}
Repair attempt ${round}/3 against pinned integration revision ${revision}.
${workspaceRules(unit)}
Assigned findings: ${JSON.stringify(findings)}.
Repair product behavior only. Reproduce each finding and mutation-prove the fix. Never edit the contract or acceptance commands. Missing dependencies are environment-gap, not invalid-check-human unless you can prove a contradictory requirement. Run affected non-browser tests, commit atomically and leave a clean worktree. Independent serial review will run all affected criteria with --log-dir before any merge; all twenty rerun on the integrated revision. Return the exact unit key ${unit.key}, base revision, branch and commit pin.`,
    { agentType: OWNERS[owner], label: `repair:${owner}:r${round}`, phase: 'Repair', schema: BUILD_SCHEMA })))
    repairRound.builds = builds
    if (human(builds) || human(calls)) return stopped('human decision required by a repair; no repairs integrated', { repairRound })
    if (!rows(builds, 'key', units.map((u) => u.unit.key))) return stopped('repair results dropped, duplicated or missing; no repairs integrated', { repairRound })
    for (const item of units) {
      item.build = builds.find((b) => b.key === item.unit.key)
      if (!buildOk(item.build, item.unit)) return stopped('a repair did not reach readiness; no repairs integrated', { repairRound })
    }
    for (const item of units) {
      const reviewLabel = `repair-${round}-${item.owner}`
      const criteria = [...new Set(item.findings.map((f) => f.key).filter((key) => REQUIRED_CRITERIA.includes(key)))]
      item.review = await ask(reviewPrompt(item.unit, item.build, criteria, reviewLabel, item.findings),
        { agentType: 'cutokyo-technical-reviewer', label: `review:repair:${item.owner}:r${round}`, phase: 'Repair', schema: REVIEW_SCHEMA })
      if (human(item.review)) return stopped('human decision required by repair review; no repairs integrated', { repairRound })
      if (!reviewOk(item.review, item.build, criteria, reviewLabel)) return stopped('independent repair review rejected or incomplete; no repairs integrated', { repairRound })
    }
    integration = await integrate(units, revision, `integrate:repair:r${round}`)
    state.integrations.push(integration)
    if (human(integration)) return stopped('human decision required by repair integration', { integration })
    if (!integrationOk(integration, units, revision)) return stopped('repair integration receipt incomplete or inconsistent', { integration })
    state.revision = integration.revision
  }
  return stopped('verification exhausted without acceptance')
}

let result
try {
  result = await run()
} catch (error) {
  result = stopped('orchestration stopped unexpectedly; retained all completed work', { error: String(error) })
}
phase('Report')
// An unsuccessful run still gets a report and retains every raw response, null,
// finding and retry in its return value. A reporter cannot promote it to pass.
const report = await ask(`${COMMON}
Inspect/create ${OUT}/reports, then allocate a fresh directory with mktemp -d ${OUT}/reports/attempt-XXXXXX. Write report.md and index.html in THAT new directory, summarizing this ${result.accepted ? 'provisionally complete' : 'UNSUCCESSFUL'} run. Never overwrite earlier reports. Return both resolved absolute paths. Do not edit source, change HEAD, commit, or alter review.json.
Pinned revision: ${state.revision || 'no valid pin established'}.
Result: ${JSON.stringify(result)}
Full evidence and attempt history: ${JSON.stringify(state)}
For every criterion state exact revision, command/observation, environment, artifact citations and limitations. Link real native/QA/JEV screenshots and action logs. Preserve every rejected/missing unit, gap and escalation. Do not turn an incomplete run into accepted. Return the assigned absolute file paths and actual revision.`,
{ agentType: 'cutokyo-architect', label: 'report', phase: 'Report', schema: REPORT_SCHEMA })
let receipt = null
let completed = result.accepted && !human(report) && report?.revision === state.revision &&
  text(report.report_path) && report.report_path.startsWith(`${OUT}/reports/attempt-`) &&
  /^[A-Za-z0-9]{6,32}\/report\.md$/.test(report.report_path.slice(`${OUT}/reports/attempt-`.length)) &&
  report.html_path === report.report_path.replace(/report\.md$/, 'index.html') && text(report.summary) && empty(report.gaps)
if (completed) {
  const final = state.history[state.history.length - 1]
  const artifacts = [...new Set([
    ...state.pin.evidence,
    ...calls.filter((c) => c.label.startsWith('pin:integrate')).flatMap((c) => c.result.units.map((u) => u.evidence)),
    ...state.candidates.flatMap((c) => [...c.build.evidence, ...c.build.commands.map((r) => r.evidence), ...c.review.personally_run.map((r) => r.evidence)]),
    ...state.repairs.flatMap((r) => r.units.flatMap((u) => [...u.build.evidence, ...u.build.commands.map((c) => c.evidence), ...u.review.personally_run.map((c) => c.evidence)])),
    ...state.integrations.flatMap((i) => i.commands.map((c) => c.evidence)),
    ...final.gates.commands.map((c) => c.evidence), ...final.gates.results.map((r) => r.evidence),
    ...final.qa.screenshots, ...final.qa.journeys.flatMap((j) => [...j.evidence, ...j.action_logs]),
    final.jev.batch, final.jev.report, ...final.jev.cases.flatMap((c) => [...c.evidence, ...c.action_logs, ...c.screenshots]),
    ...final.review.criteria.map((c) => c.citation), report.report_path, report.html_path,
  ])]
  receipt = await ask(`${COMMON}
Independent final physical-evidence audit. You did not build or write the report. Read BOTH final reports and EVERY artifact in this exact manifest:
${JSON.stringify(artifacts)}
Expected final HEAD: ${state.revision}. Run git rev-parse HEAD and git status --porcelain in ${ROOT} before and after inspection; require clean main at that exact SHA. Compare review.json bytes and acceptance commands to ${CONTRACT_BASE}. Confirm all reviewed candidate/repair pins remain ancestors of HEAD. Independently read raw logs, runner receipts AND all segment logs they reference, screenshots and action logs; require runner state=complete, status=pass, exit_code=0, command_unmodified=true, verified immutable contract identity, clean tracked_status and unchanged source_after. Verify actual successful test execution, criterion coverage, revision/environment identity and no outstanding gaps or contradictions. A killed or incomplete runner cannot pass even if its last visible test was green. Prior candidate/repair evidence must match its own reviewed pin; final gates/QA/JEV/criterion citations and reports must all establish ${state.revision}. Do not infer identity just from a filename or a report boolean.
Evidence context: ${JSON.stringify({ candidates: state.candidates, repairs: state.repairs, integrations: state.integrations, final, report })}
For EACH exact manifest path report readable=true only after opening actual nonempty content and compute its SHA-256. Read screenshots into context, not merely stat files. Any absent/unreadable/stale/unattributable artifact fails. Return the actual HEAD, clean, contract_unchanged, evidence_matches_revision, exact unique artifact receipts, findings and gaps. Read-only, no repairs.`,
  { agentType: 'cutokyo-technical-reviewer', label: 'receipt', phase: 'Report', schema: RECEIPT_SCHEMA })
  completed = !human(receipt) && receipt?.revision === state.revision && receipt.clean === true &&
    receipt.contract_unchanged === true && receipt.evidence_matches_revision === true && empty(receipt.findings) && empty(receipt.gaps) &&
    rows(receipt.artifacts, 'path', artifacts) && receipt.artifacts.every((a) => a.readable === true && /^[0-9a-f]{64}$/.test(a.sha256))
}
return {
  ...result, accepted: Boolean(completed),
  halted: completed ? null : result.halted || 'final report or independent physical-evidence receipt incomplete',
  needsHuman: Boolean(result.needsHuman || human(report) || human(receipt)),
  ...state, report, receipt,
}

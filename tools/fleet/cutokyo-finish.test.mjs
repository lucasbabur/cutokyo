import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

// Execute the saved workflow itself, not a copied predicate or source-text test.
// Only its export declaration changes for the same async script context used by
// the workflow sandbox. No real agents, workflows, product commands or merges run.
const source = await readFile(new URL('../../.claude/workflows/cutokyo-finish.js', import.meta.url), 'utf8')
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor
const execute = new AsyncFunction('agent', 'parallel', 'pipeline', 'phase', 'log', 'args', source.replace(/^export const meta = /, 'const meta = '))
const ROOT = '/home/lucas/Developer/personal/cutokyo-community'
const OUT = '/home/lucas/Developer/personal/cutokyo-community-finish-evidence/regression'
const FLEET = `${ROOT}/.claude/fleets/20260919-cutokyo-v01`
const BASE = '1db95d8659d31df8cea6933a994d9a196205bf8e'
const CRITERIA = Array.from({ length: 20 }, (_, i) => `C${String(i + 1).padStart(2, '0')}`)
// Independent expected lists from the original 27-journey/eight-case contract.
const JOURNEYS = [
  'onboarding-setup-rollback', 'uninstall-never-activated', 'restore-no-state-repeat',
  'restore-partial-concurrent-edit', 'settings-contract-preservation', 'empty-history',
  'populated-history', 'search-resume-claude', 'search-resume-codex', 'search-resume-opencode',
  'retention-delete', 'dashboard-reconciliation', 'inventory', 'mcp-failure-isolation',
  'plugin-verification', 'guards', 'proxy-consent-degraded', 'analysis-consent-cancel-retry',
  'health-lock-quarantine', 'health-persistence-restart', 'health-current-lifetime-large-history',
  'doctor-bundle-backup', 'backup-tamper-restore', 'artifact-source-immutability',
  'updater-choice', 'accessibility-keyboard', 'native-tauri-startup',
]
const CASES = [
  'onboarding-empty-history', 'search-detail-resume', 'retention-delete-confirmation',
  'guard-proxy-coverage-language', 'degraded-health-recovery',
  'mcp-plugin-inventory', 'visual-keyboard-consistency',
]
const sha = (s) => createHash('sha1').update(s).digest('hex')
const command = (text, evidence = `${OUT}/evidence/command.log`) => ({ command: text, exit_code: 0, evidence })
const finding = (key = 'C02', owner = 'core') => ({ key, owner, defect: 'Observed production defect', location: '/source/file.rs:10', reproduction: 'Run failing scenario', blocking: true })
const staticCommands = (base) => [
  `python3 ${ROOT}/tools/fleet/cutokyo-gates.py selftest`,
  `python3 ${ROOT}/tools/fleet/cutokyo-gates.py all --root ${ROOT}`,
  'python3 /home/lucas/.claude/fleet-tools/diffguard/diffguard.py selftest',
  `python3 /home/lucas/.claude/fleet-tools/diffguard/diffguard.py scan --repo ${ROOT} --base ${base} --json`,
].map((c, i) => command(c, `${OUT}/evidence/${sha(base)}/static-${i}.log`))
const jsonLine = (prompt, prefix) => JSON.parse(prompt.split('\n').find((s) => s.startsWith(prefix)).slice(prefix.length).replace(/\.$/, ''))
function acceptanceCommands(prompt) {
  return prompt.split('\n').filter((s) => s.startsWith(`python3 ${ROOT}/tools/fleet/cutokyo-acceptance.py run `) && !s.includes('<ID>')).map((s) => {
    const resolved = s.replace(/attempt-XXXXXX$/, 'attempt-fixture')
    const id = resolved.match(/--criterion (C\d\d)/)[1]
    const dir = resolved.match(/--log-dir (\S+)/)[1]
    return command(resolved, `${dir}/${id}-report.json`)
  })
}

// The mock checks schema requirements/types on the good fixture before injecting
// faults. Malformed responses deliberately bypass that check to exercise the
// workflow's own fail-closed decisions, including null/dropped runtime results.
function validate(value, schema, at = '') {
  if (schema.enum) assert.ok(schema.enum.includes(value), at)
  if (schema.type === 'object') {
    assert.ok(value && typeof value === 'object' && !Array.isArray(value), at)
    for (const key of schema.required || []) assert.ok(Object.hasOwn(value, key), `${at}.${key} required`)
    for (const [key, child] of Object.entries(schema.properties)) if (Object.hasOwn(value, key)) validate(value[key], child, `${at}.${key}`)
  } else if (schema.type === 'array') {
    assert.ok(Array.isArray(value), at)
    value.forEach((v, i) => validate(v, schema.items, `${at}[${i}]`))
  } else if (schema.type === 'integer') assert.ok(Number.isInteger(value), at)
  else assert.equal(typeof value, schema.type, at)
}
async function run({ mutate = (_, r) => r, dropParallel, args = { runKey: 'regression' } } = {}) {
  const events = [], prompts = [], logs = [], phases = [], integrations = []
  const built = new Map()
  let head = BASE, browserActive = 0, maxBrowserActive = 0, buildActive = 0, maxBuildActive = 0, parallelRound = 0
  const agent = async (prompt, options) => {
    const label = options.label
    prompts.push({ label, prompt, options })
    events.push(`start:${label}`)
    const browser = /^(gates:|qa:|jev:|review:(C|repair:))/.test(label)
    const builder = /^(build:|repair:)/.test(label)
    if (browser) maxBrowserActive = Math.max(maxBrowserActive, ++browserActive)
    if (builder) maxBuildActive = Math.max(maxBuildActive, ++buildActive)
    await new Promise((resolve) => setImmediate(resolve))
    let result
    try {
      if (label === 'pin') {
        result = { revision: head, clean: true, contract_unchanged: true, evidence: [`${OUT}/evidence/preflight.log`], gaps: [] }
      } else if (builder) {
        const unit = jsonLine(prompt, 'Use ONLY this workspace: ')
        assert.ok(!unit.worktree.startsWith('/tmp/'))
        assert.notEqual(unit.branch, 'main')
        const createCommand = prompt.split('\n').find((line) => line.startsWith(`  git -C ${ROOT} worktree add -b `))
        assert.deepEqual(createCommand.trim().split(/\s+/), ['git', '-C', ROOT, 'worktree', 'add', '-b', unit.branch, unit.worktree, unit.base])
        result = {
          key: unit.key, done: true, branch: unit.branch, base_revision: unit.base,
          revision: sha(label), worktree: unit.worktree, commits: [sha(label)], files_changed: ['source.rs'],
          commands: [command('cargo test')], mutations: ['intentional regression failed before restoration'],
          evidence: [`${OUT}/evidence/${label.replaceAll(':', '-')}.log`], gaps: [], notes: 'Ready for review',
        }
      } else if (/^review:(C|repair:)/.test(label)) {
        const key = label.startsWith('review:repair:') ? `repair-${label.split(':').at(-1).slice(1)}-${label.split(':')[2]}` : label.split(':')[1]
        const build = built.get(key)
        const personally_run = acceptanceCommands(prompt)
        if (!personally_run.length) {
          const template = prompt.split('\n').find((s) => s.startsWith(`python3 ${ROOT}/tools/fleet/cutokyo-acceptance.py run `) && s.includes('<ID>'))
          const c = template.replaceAll('<ID>', 'C17').replace(/attempt-XXXXXX$/, 'attempt-fixture')
          personally_run.push(command(c, `${c.match(/--log-dir (\S+)/)[1]}/C17-report.json`))
        }
        result = {
          key, verdict: 'pass', revision: build.revision, contract_unchanged: true, ran_acceptance: true,
          findings: [], personally_run, criteria_run: personally_run.map((c) => c.command.match(/--criterion (C\d\d)/)[1]), gaps: [], summary: 'Independently verified',
        }
      } else if (label.startsWith('pin:integrate')) {
        result = {
          revision: head, clean: true, contract_unchanged: true, gaps: [],
          units: jsonLine(prompt, 'Approved inputs: ').map((u) => ({ ...u, clean: true, acceptance_verified: true, evidence: `${OUT}/evidence/pins/${u.key}.log` })),
        }
      } else if (label.startsWith('integrate')) {
        const units = JSON.parse(prompt.split('\n').find((s) => s.startsWith('[{"key":')))
        const mergeCommands = prompt.split('\n').filter((line) => line.startsWith(`git -C ${ROOT} merge `))
        assert.deepEqual(mergeCommands, units.map((u) => `git -C ${ROOT} merge --no-ff --no-commit ${u.revision}`))
        result = {
          done: true, base_revision: head, revision: sha(label), merged_branches: units.map((u) => u.branch),
          merged_revisions: units.map((u) => u.revision), conflicts: [], commands: staticCommands(head), gaps: [],
        }
      } else if (label.startsWith('gates:')) {
        result = {
          selftests_ok: true, all_passed: true, revision: head, commands: staticCommands(BASE), gaps: [],
          results: acceptanceCommands(prompt).map((c) => ({ ...c, key: c.command.match(/--criterion (C\d\d)/)[1], status: 'pass', owner: 'core', diagnostic: 'passed' })),
        }
      } else if (label.startsWith('qa:')) {
        result = {
          verdict: 'pass', revision: head, native_app_tested: true, installed_harnesses: ['claude', 'codex', 'opencode'],
          screenshots: [`${OUT}/evidence/${label}/native.png`],
          journeys: JOURNEYS.map((key) => ({ key, status: 'pass', evidence: [`${OUT}/evidence/${label}/${key}.json`], action_logs: [`${OUT}/evidence/${label}/${key}.log`], summary: 'Personally driven' })),
          failures: [], gaps: [], summary: 'All required journeys exercised',
        }
      } else if (label.startsWith('jev:')) {
        result = {
          verdict: 'pass', revision: head, validated: true, screenshots_read: true,
          batch: `${FLEET}/jev-cases.yaml`, report: `${OUT}/evidence/${label}/report.json`,
          cases: CASES.map((key) => ({ key, status: 'PASS', evidence: [`${OUT}/evidence/${label}/${key}.json`], action_logs: [`${OUT}/evidence/${label}/${key}.log`], screenshots: [`${OUT}/evidence/${label}/${key}.png`], summary: 'Read screenshot and actions' })),
          failures: [], gaps: [], summary: 'Exact validated batch passed',
        }
      } else if (/^review:r\d+$/.test(label)) {
        result = {
          verdict: 'pass', revision: head,
          criteria: CRITERIA.map((id) => ({ id, verdict: 'pass', citation: `${OUT}/evidence/${label}/${id}.json`, basis: 'Artifact observation at source.rs:10' })),
          findings: [], gaps: [], summary: 'Every criterion cited',
        }
      } else if (label === 'report') {
        result = { revision: head, report_path: `${OUT}/reports/attempt-fixture/report.md`, html_path: `${OUT}/reports/attempt-fixture/index.html`, summary: 'Honest evidence and limitations', gaps: [] }
      } else if (label === 'receipt') {
        result = {
          revision: head, clean: true, contract_unchanged: true, evidence_matches_revision: true,
          artifacts: JSON.parse(prompt.split('\n').find((s) => s.startsWith('["/'))).map((path) => ({ path, readable: true, sha256: createHash('sha256').update(path).digest('hex') })),
          findings: [], gaps: [],
        }
      } else assert.fail(`Unexpected agent ${label}`)
      validate(result, options.schema, label)
      result = await mutate(label, result, { prompt, built, head, events, integrations })
      if (builder && result) built.set(result.key, structuredClone(result))
      if (label.startsWith('integrate')) {
        integrations.push({ label, result, prompt })
        if (result?.done) head = result.revision
      }
      return result
    } finally {
      if (browser) browserActive--
      if (builder) buildActive--
      events.push(`end:${label}`)
    }
  }
  const parallel = async (thunks) => {
    const results = await Promise.all(thunks.map(async (f) => { try { return await f() } catch { return null } }))
    parallelRound++
    return dropParallel ? dropParallel(results, parallelRound) : results
  }
  const pipeline = async (items, ...stages) => Promise.all(items.map(async (item, index) => {
    let value = item
    try { for (const stage of stages) value = await stage(value, item, index) } catch { return null }
    return value
  }))
  const result = await execute(agent, parallel, pipeline, (p) => phases.push(p), (s) => logs.push(s), args)
  return { result, events, prompts, logs, phases, integrations, maxBrowserActive, maxBuildActive }
}
function change(target, fn) {
  return (label, result, context) => label === target ? fn(result, context) ?? result : result
}
function assertStopped(run, merges) {
  assert.equal(run.result.accepted, false)
  assert.equal(run.integrations.length, merges, 'no additional integration was authorized')
  assert.ok(run.result.halted)
  assert.ok(run.result.report, 'unsuccessful evidence report retained')
  assert.ok(run.result.calls.length)
}
function productFailure(label, result) {
  if (label === 'gates:r1') {
    result.all_passed = false
    Object.assign(result.results[1], { status: 'fail', exit_code: 1, diagnostic: 'Real C02 defect', owner: 'core' })
  }
  return result
}

test('complete reports pass the actual workflow with all original keys and independent receipt', async () => {
  const r = await run()
  assert.equal(r.result.accepted, true)
  assert.equal(r.integrations.length, 1)
  assert.equal(r.maxBrowserActive, 1, 'gates, QA, JEV and acceptance reviews must never overlap')
  assert.equal(r.maxBuildActive, 3, 'candidate builds remain parallel')
  assert.deepEqual(r.result.history[0].gates.results.map((v) => v.key), CRITERIA)
  assert.deepEqual(r.result.history[0].qa.journeys.map((v) => v.key), JOURNEYS)
  assert.deepEqual(r.result.history[0].jev.cases.map((v) => v.key), CASES)
  for (const pair of [['gates:r1', 'qa:r1'], ['qa:r1', 'jev:r1'], ['jev:r1', 'review:r1'], ['report', 'receipt']]) {
    assert.ok(r.events.indexOf(`end:${pair[0]}`) < r.events.indexOf(`start:${pair[1]}`))
  }
  assert.ok(r.result.receipt.artifacts.length > 100)
  for (const artifact of r.result.receipt.artifacts) {
    if (artifact.path === `${FLEET}/jev-cases.yaml`) continue
    assert.ok(!artifact.path.startsWith(`${ROOT}/`), 'generated evidence must not dirty the source tree')
    assert.ok(!artifact.path.startsWith('/tmp/'), 'evidence must survive reboot')
  }
  assert.equal(r.result.halted, null)
})

for (const label of ['pin', 'build:C16:r1', 'review:C16:r1', 'pin:integrate', 'integrate', 'gates:r1', 'qa:r1', 'jev:r1', 'review:r1', 'report', 'receipt']) {
  test(`${label}: missing/null response cannot accept or integrate further`, async () => {
    const r = await run({ mutate: (l, v) => l === label ? null : v })
    assert.equal(r.result.accepted, false)
    assert.equal(r.integrations.length, ['pin', 'build:C16:r1', 'review:C16:r1', 'pin:integrate'].includes(label) ? 0 : 1)
    assert.ok(r.result.halted)
    assert.ok(r.result.calls.some((c) => c.label === label && c.result === null))
  })
}

for (const label of ['review:C16:r1', 'gates:r1', 'qa:r1', 'jev:r1', 'review:r1', 'report', 'receipt']) {
  test(`${label}: stale full revision fails closed`, async () => {
    const r = await run({ mutate: change(label, (v) => { v.revision = sha('stale') }) })
    assertStopped(r, label === 'review:C16:r1' ? 0 : 1)
  })
  test(`${label}: abbreviated revision is not a pin`, async () => {
    const r = await run({ mutate: change(label, (v) => { v.revision = v.revision.slice(0, 7) }) })
    assertStopped(r, label === 'review:C16:r1' ? 0 : 1)
  })
}

for (const [label, field, id] of [['gates:r1', 'results', 'key'], ['qa:r1', 'journeys', 'key'], ['jev:r1', 'cases', 'key'], ['review:r1', 'criteria', 'id']]) {
  for (const fault of ['empty', 'missing', 'duplicate', 'extra', 'unknown', 'all-duplicate']) {
    test(`${label}: ${fault} IDs rejected`, async () => {
      const r = await run({ mutate: change(label, (v) => {
        if (fault === 'empty') v[field] = []
        if (fault === 'missing') v[field].pop()
        if (fault === 'duplicate') v[field][1] = structuredClone(v[field][0])
        if (fault === 'extra') v[field].push(structuredClone(v[field][0]))
        if (fault === 'unknown') v[field][0][id] = 'unrecognized'
        if (fault === 'all-duplicate') v[field] = v[field].map(() => structuredClone(v[field][0]))
      }) })
      assertStopped(r, 1)
      assert.equal(r.result.repairs.length, 0, 'malformed evidence must not trigger a product repair')
    })
  }
}

const faults = [
  ['gates:r1', 'failed exit under pass status', (v) => { v.results[0].exit_code = 1 }],
  ['gates:r1', 'zero-test exit 86', (v) => { v.results[0].exit_code = 86 }],
  ['gates:r1', 'string exit code', (v) => { v.results[0].exit_code = '0' }],
  ['gates:r1', 'fail status under all_passed', (v) => { v.results[0].status = 'fail'; v.results[0].exit_code = 1 }],
  ['gates:r1', 'warn is not pass', (v) => { v.results[0].status = 'warn' }],
  ['gates:r1', 'void is not pass', (v) => { v.results[0].status = 'void' }],
  ['gates:r1', 'missing selftest evidence', (v) => { v.commands = [] }],
  ['gates:r1', 'invented static command', (v) => { v.commands[0].command = 'true' }],
  ['gates:r1', 'wrong acceptance command', (v) => { v.results[0].command = 'true' }],
  ['gates:r1', 'unresolved attempt template', (v) => { v.results[0].command = v.results[0].command.replace('attempt-fixture', 'attempt-XXXXXX'); v.results[0].evidence = v.results[0].evidence.replace('attempt-fixture', 'attempt-XXXXXX') }],
  ['gates:r1', 'attempt directory traversal', (v) => { v.results[0].command = v.results[0].command.replace('attempt-fixture', 'attempt-../old'); v.results[0].evidence = v.results[0].evidence.replace('attempt-fixture', 'attempt-../old') }],
  ['gates:r1', 'missing raw receipt', (v) => { v.results[0].evidence = '' }],
  ['gates:r1', 'wrong receipt path', (v) => { v.results[0].evidence = '/old/log.json' }],
  ['gates:r1', 'whitespace evidence', (v) => { v.results[0].evidence = '  ' }],
  ['qa:r1', 'empty journey evidence', (v) => { v.journeys[0].evidence = [] }],
  ['qa:r1', 'blank journey evidence', (v) => { v.journeys[0].evidence = [' '] }],
  ['qa:r1', 'missing action logs', (v) => { v.journeys[0].action_logs = [] }],
  ['qa:r1', 'missing screenshots', (v) => { v.screenshots = [] }],
  ['qa:r1', 'no native application', (v) => { v.native_app_tested = false }],
  ['qa:r1', 'missing installed harness', (v) => { v.installed_harnesses.pop() }],
  ['qa:r1', 'three duplicate installed harnesses', (v) => { v.installed_harnesses = ['claude', 'claude', 'claude'] }],
  ['qa:r1', 'unknown installed harness', (v) => { v.installed_harnesses[0] = 'other' }],
  ['qa:r1', 'blocked journey under passing report', (v) => { v.journeys[0].status = 'blocked' }],
  ['qa:r1', 'outstanding failure under pass', (v) => { v.failures.push(finding()) }],
  ['qa:r1', 'outstanding gap', (v) => { v.gaps.push('unverified native package') }],
  ['jev:r1', 'empty action evidence', (v) => { v.cases[0].evidence = [] }],
  ['jev:r1', 'empty action logs', (v) => { v.cases[0].action_logs = [] }],
  ['jev:r1', 'blank screenshots', (v) => { v.cases[0].screenshots = [''] }],
  ['jev:r1', 'screenshots not read', (v) => { v.screenshots_read = false }],
  ['jev:r1', 'unvalidated batch', (v) => { v.validated = false }],
  ['jev:r1', 'missing batch', (v) => { v.batch = '' }],
  ['jev:r1', 'missing report', (v) => { v.report = '' }],
  ['jev:r1', 'ERROR under passing report', (v) => { v.cases[0].status = 'ERROR' }],
  ['jev:r1', 'duplicate BLOCKED case', (v) => { v.cases.push({ ...v.cases[0], status: 'BLOCKED' }) }],
  ['jev:r1', 'outstanding failure under pass', (v) => { v.failures.push(finding()) }],
  ['jev:r1', 'outstanding gap', (v) => { v.gaps.push('unreadable action trace') }],
  ['review:r1', 'blank citation', (v) => { v.criteria[0].citation = ' ' }],
  ['review:r1', 'blank basis', (v) => { v.criteria[0].basis = '' }],
  ['review:r1', 'criterion failure under pass', (v) => { v.criteria[0].verdict = 'fail' }],
  ['review:r1', 'outstanding blocking finding', (v) => { v.findings.push(finding()) }],
  ['review:r1', 'outstanding nonblocking finding', (v) => { v.findings.push({ ...finding(), blocking: false }) }],
  ['review:r1', 'outstanding gap', (v) => { v.gaps.push('not inspected') }],
  ['report', 'blank report path', (v) => { v.report_path = '' }],
  ['report', 'wrong HTML path', (v) => { v.html_path = '/different.html' }],
  ['report', 'blank summary', (v) => { v.summary = ' ' }],
  ['report', 'outstanding gap', (v) => { v.gaps.push('report incomplete') }],
  ['receipt', 'dirty final tree', (v) => { v.clean = false }],
  ['receipt', 'mutated contract', (v) => { v.contract_unchanged = false }],
  ['receipt', 'unattributable evidence', (v) => { v.evidence_matches_revision = false }],
  ['receipt', 'unreadable artifact', (v) => { v.artifacts[0].readable = false }],
  ['receipt', 'missing artifact', (v) => { v.artifacts.pop() }],
  ['receipt', 'duplicate artifact', (v) => { v.artifacts[1] = v.artifacts[0] }],
  ['receipt', 'blank digest', (v) => { v.artifacts[0].sha256 = '' }],
  ['receipt', 'unresolved finding', (v) => { v.findings.push(finding()) }],
]
for (const [label, name, fault] of faults) {
  test(`${label}: ${name}`, async () => assertStopped(await run({ mutate: change(label, fault) }), 1))
}

for (const [name, fault] of [
  ['ran_acceptance=false', (v) => { v.ran_acceptance = false }],
  ['contract changed', (v) => { v.contract_unchanged = false }],
  ['no personal commands', (v) => { v.personally_run = [] }],
  ['personal command failed', (v) => { v.personally_run[0].exit_code = 1 }],
  ['personal evidence blank', (v) => { v.personally_run[0].evidence = '' }],
  ['personal command invented', (v) => { v.personally_run[0].command = 'true' }],
  ['wrong criterion', (v) => { v.criteria_run = ['C01'] }],
  ['blocking finding remains', (v) => { v.findings.push(finding()) }],
  ['nonblocking finding remains', (v) => { v.findings.push({ ...finding(), blocking: false }) }],
  ['gap remains', (v) => { v.gaps.push('missing dependency') }],
]) {
  test(`candidate approval rejected: ${name}`, async () => assertStopped(await run({ mutate: change('review:C16:r1', fault) }), 0))
}

for (const [name, fault] of [
  ['done false', (v) => { v.done = false }],
  ['wrong worktree', (v) => { v.worktree = '/tmp/unsafe' }],
  ['main branch', (v) => { v.branch = 'main' }],
  ['wrong base', (v) => { v.base_revision = sha('other-base') }],
  ['short revision', (v) => { v.revision = '1234567' }],
  ['no commands', (v) => { v.commands = [] }],
  ['no evidence', (v) => { v.evidence = [] }],
]) {
  test(`candidate build rejected: ${name}`, async () => assertStopped(await run({ mutate: change('build:C19:r1', fault) }), 0))
}

for (const [name, fault] of [
  ['moved branch head', (v) => { v.units[0].revision = sha('unreviewed') }],
  ['moved main', (v) => { v.revision = sha('new-main') }],
  ['dirty candidate', (v) => { v.units[0].clean = false }],
  ['unverified physical acceptance receipt', (v) => { v.units[0].acceptance_verified = false }],
  ['missing candidate', (v) => { v.units.pop() }],
  ['mutated contract', (v) => { v.contract_unchanged = false }],
]) {
  test(`premerge pin check blocks ${name} before merge authorization`, async () => assertStopped(await run({ mutate: change('pin:integrate', fault) }), 0))
}
for (const [name, fault] of [
  ['dropped branch', (v) => { v.merged_branches.pop() }],
  ['unreviewed moved SHA', (v) => { v.merged_revisions[0] = sha('moved') }],
  ['wrong integration base', (v) => { v.base_revision = sha('other') }],
  ['short integrated revision', (v) => { v.revision = '1234567' }],
  ['remaining conflict', (v) => { v.conflicts.push('source.rs') }],
  ['failed command', (v) => { v.commands[0].exit_code = 1 }],
  ['missing static check', (v) => { v.commands.pop() }],
]) {
  test(`integration receipt blocks ${name}`, async () => {
    const r = await run({ mutate: change('integrate', fault) })
    assertStopped(r, 1)
    assert.equal(r.result.history.length, 0)
  })
}

test('candidate rejection is bounded to three attempts and blocks all integration', async () => {
  const r = await run({ mutate: (label, v) => {
    if (/^review:C19:/.test(label)) { v.verdict = 'fail'; v.findings = [finding('C19', 'cli-ops')] }
    return v
  } })
  assertStopped(r, 0)
  assert.equal(r.prompts.filter((p) => /^build:C19:/.test(p.label)).length, 3)
  assert.equal(r.prompts.filter((p) => /^build:C16:/.test(p.label)).length, 1)
  assert.match(r.result.halted, /limit/)
})

test('one failed candidate can be repaired and independently approved without discarding prior rejection', async () => {
  const r = await run({ mutate: change('review:C19:r1', (v) => { v.verdict = 'fail'; v.findings = [finding('C19', 'cli-ops')] }) })
  assert.equal(r.result.accepted, true)
  assert.ok(r.result.calls.some((c) => c.label === 'review:C19:r1' && c.result.verdict === 'fail'))
  assert.equal(r.result.candidates.find((c) => c.key === 'C19').attempts, 2)
})

for (const drop of [(r) => r.slice(1), (r) => r.map((v, i) => i ? v : null)]) {
  test('dropped parallel candidate result cannot be filtered into partial integration', async () => assertStopped(await run({ dropParallel: drop }), 0))
}
test('thrown candidate is retained as a null/error and blocks integration', async () => {
  const r = await run({ mutate: (label, v) => { if (label === 'build:C19:r1') throw new Error('terminal API error'); return v } })
  assertStopped(r, 0)
  assert.match(r.result.calls.find((c) => c.label === 'build:C19:r1').error, /terminal API error/)
})

for (const label of ['build:C19:r1', 'review:C16:r1', 'pin:integrate', 'integrate', 'gates:r1', 'qa:r1', 'jev:r1', 'review:r1', 'report', 'receipt']) {
  test(`invalid-check-human from ${label} is surfaced despite other successes`, async () => {
    const r = await run({ mutate: change(label, (v) => { v.gaps.push('invalid-check-human: exact contradiction with evidence /proof/log') }) })
    assert.equal(r.result.accepted, false)
    assert.equal(r.result.needsHuman, true)
    assert.equal(r.integrations.length, ['build:C19:r1', 'review:C16:r1', 'pin:integrate'].includes(label) ? 0 : 1)
  })
}
test('a missing dependency is an environment gap, not an invalid requirement', async () => {
  const r = await run({ mutate: change('gates:r1', (v) => {
    v.gaps = ['environment-gap: WebKit development package unavailable']
    v.results[16].status = 'void'; v.results[16].exit_code = 127; v.all_passed = false
  }) })
  assertStopped(r, 1)
  assert.equal(r.result.needsHuman, false)
  assert.match(r.result.history[0].gates.gaps[0], /environment-gap/)
})

test('product repair starts from the pinned integration revision, is independently reviewed, then reverified', async () => {
  const r = await run({ mutate: productFailure })
  assert.equal(r.result.accepted, true)
  assert.equal(r.integrations.length, 2)
  assert.equal(r.result.repairs.length, 1)
  const repair = r.result.repairs[0].units[0]
  assert.equal(repair.unit.base, sha('integrate'))
  assert.equal(repair.build.base_revision, sha('integrate'))
  assert.notEqual(repair.unit.branch, 'main')
  assert.ok(repair.unit.worktree.startsWith('/home/lucas/Developer/personal/'))
  assert.ok(r.events.indexOf('end:review:repair:core:r1') < r.events.indexOf('start:integrate:repair:r1'))
  assert.equal(r.result.history[1].revision, sha('integrate:repair:r1'))
  assert.equal(r.maxBrowserActive, 1)
})

for (const fault of ['fail', 'null', 'stale', 'empty-evidence', 'human']) {
  test(`repair independent review ${fault} prohibits repair integration`, async () => {
    const r = await run({ mutate: (label, v) => {
      productFailure(label, v)
      if (label === 'review:repair:core:r1') {
        if (fault === 'null') return null
        if (fault === 'fail') { v.verdict = 'fail'; v.findings.push(finding()) }
        if (fault === 'stale') v.revision = sha('stale')
        if (fault === 'empty-evidence') v.personally_run = []
        if (fault === 'human') v.gaps.push('invalid-check-human: contradiction /proof/log')
      }
      return v
    } })
    assertStopped(r, 1)
    assert.equal(r.result.needsHuman, fault === 'human')
  })
}
for (const fault of ['dropped', 'null', 'unready', 'human', 'wrong-base']) {
  test(`one ${fault} repair alongside a successful repair prevents partial merge`, async () => {
    const r = await run({
      mutate: (label, v) => {
        productFailure(label, v)
        if (label === 'gates:r1') Object.assign(v.results[16], { status: 'fail', exit_code: 1, owner: 'desktop', diagnostic: 'Real C17 defect' })
        if (label === 'repair:desktop:r1') {
          if (fault === 'null') return null
          if (fault === 'unready') v.done = false
          if (fault === 'human') v.gaps.push('invalid-check-human: contradiction /proof/log')
          if (fault === 'wrong-base') v.base_revision = BASE
        }
        return v
      },
      dropParallel: fault === 'dropped' ? (v, round) => round === 2 ? v.slice(0, 1) : v : undefined,
    })
    assertStopped(r, 1)
    assert.equal(r.result.needsHuman, fault === 'human')
  })
}
test('all four verification rounds retain findings and stop after three repairs', async () => {
  const r = await run({ mutate: (label, v) => {
    if (/^gates:r\d+$/.test(label)) { v.all_passed = false; Object.assign(v.results[1], { status: 'fail', exit_code: 1, owner: 'core' }) }
    return v
  } })
  assertStopped(r, 4)
  assert.equal(r.result.history.length, 4)
  assert.equal(r.result.repairs.length, 3)
  assert.match(r.result.halted, /limit/)
  assert.ok(r.logs.some((l) => l.includes('at most three repair attempts')))
})
test('failed journey without its own finding is not silently discarded for another failure', async () => {
  const r = await run({ mutate: (label, v) => {
    productFailure(label, v)
    if (label === 'qa:r1') { v.verdict = 'fail'; v.journeys[0].status = 'fail' }
    return v
  } })
  assertStopped(r, 1)
  assert.equal(r.result.repairs.length, 0)
})
test('stale second-round evidence is not reused after repair integration', async () => {
  const r = await run({ mutate: (label, v) => {
    productFailure(label, v)
    if (label === 'qa:r2') v.revision = sha('integrate')
    return v
  } })
  assertStopped(r, 2)
})
test('fresh resolved attempt directories accept without reusing earlier receipt paths', async () => {
  const r = await run({ mutate: (label, v) => {
    for (const c of v.personally_run || v.results || []) {
      c.command = c.command.replace('attempt-fixture', 'attempt-fresh42')
      c.evidence = c.evidence.replace('attempt-fixture', 'attempt-fresh42')
    }
    if (label === 'report') {
      v.report_path = v.report_path.replace('attempt-fixture', 'attempt-fresh42')
      v.html_path = v.html_path.replace('attempt-fixture', 'attempt-fresh42')
    }
    return v
  } })
  assert.equal(r.result.accepted, true)
  assert.match(r.result.report.report_path, /attempt-fresh42/)
})
test('unexpected orchestration failure preserves completed work and writes a failure report', async () => {
  const r = await run({ dropParallel: () => { throw new Error('parallel runtime stopped') } })
  assertStopped(r, 0)
  assert.match(r.result.error, /parallel runtime stopped/)
  assert.equal(r.result.calls.filter((c) => c.label.startsWith('build:')).length, 3)
})
test('invalid run namespace stops before any agent or worktree instruction', async () => {
  for (const args of [undefined, {}, { runKey: '../../main' }, { runKey: 'bad;command' }]) {
    const r = await run({ args: args || {} })
    assert.equal(r.result.accepted, false)
    assert.equal(r.events.length, 0)
  }
})

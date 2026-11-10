export const meta = {
  name: 'provider-step',
  description: 'Implement one provider-refactor migration step in its worktree, review it twice, fix',
  phases: [
    { title: 'Implement', detail: 'one agent in the step worktree' },
    { title: 'Review', detail: 'two independent reviewers' },
    { title: 'Fix', detail: 'apply confirmed findings, re-run checks, commit' },
    { title: 'Recheck', detail: 'review the fix commits, fix what survives' },
    { title: 'Gate', detail:'CRAP gate and full suites once, before the PR' },
  ],
}

const MAIN = '/home/user/unshitty-mac-filebrowser'
const WT = MAIN + '/.worktrees/' + args.worktree
const SPEC = WT + '/docs/superpowers/specs/2026-11-10-provider-crates-design.md'
const RULES = [
  'Repository rules: ' + WT + '/AGENTS.md (read it). Work ONLY in the worktree ' + WT + ' (branch ' + args.branch + ', from origin/main). Never run npm ci/install in the worktree. Never push, open PRs or merge. Read the main checkout ' + MAIN + ' only.',
  'Disk is tight (about 11 GB free; check with df -h /). Use CARGO_TARGET_DIR=' + MAIN + '/src-tauri/target for cargo build/test, one build at a time. KEEP this cache warm: never cargo clean pre-emptively, but if df -h / shows under 5 GB free, delete target/debug/incremental and stale filebrowser_lib-*/filebrowser-* test binaries in target/debug/deps (keep the newest); while iterating run only the crates and tests you touch (cargo test --locked -p <crate> <filter>), and the full cargo test --locked once before your last commit. If a build fails with "No space left on device", clean both targets and retry. "npm run build" must run before cargo in src-tauri (generate_context reads ../dist).',
  'Commit with a conventional message whose body explains what and why, ending with exactly:\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_01JuD4cHzwTaYyoVEqpxTMnV\nOWNER DECISION: commit with git commit --no-verify (the owner waived the per-commit hook); the CRAP gate runs once on the finished branch before the PR, in a final phase. Still run the checks this prompt asks for before committing.',
  'The owner has asked for full automation: take the decisions the spec leaves open, the safest one that keeps behaviour identical, and record each in the commit message. If the spec turns out wrong for this step, follow the code, and correct the spec text in the same branch.',
].join('\n')

const RULES_FAST = [
  'Repository rules: ' + WT + '/AGENTS.md (read it). Work ONLY in the worktree ' + WT + ' (branch ' + args.branch + ', from origin/main). Never run npm ci/install in the worktree. Never push, open PRs or merge. Read the main checkout ' + MAIN + ' only.',
  'SPEED RULES (builds dominate the time): KEEP build caches warm. While iterating, compile and run only what you touch, e.g. CARGO_TARGET_DIR=' + MAIN + '/src-tauri/target cargo test --locked -p <crate> <test filter>; never cargo clean pre-emptively. The pre-commit hook needs about 10 GB for its coverage build in the worktree target: just before git commit, check df -h /; only if under 12 GB free, run cargo clean on the shared target ' + MAIN + '/src-tauri/target. Do not clean the worktree target between your own commits; clean it (cargo clean --manifest-path ' + WT + '/src-tauri/Cargo.toml) only once, at the very end. If a build fails with No space left on device, clean the shared target and retry. npm run build must run before cargo in src-tauri (generate_context reads ../dist).',
  ...RULES.split('\n').slice(2),
].join('\n')

const STEP ='Implement MIGRATION STEP ' + args.step + ' of the design spec ' + SPEC + ' (read the whole spec and the code it names first; step 1 is merged: the places crate at src-tauri/crates/places holds the shared types, Provider, Transfer, Extensions, Registry, Error, Capabilities). This step:\n' + args.task + '\n\nHard rules for every step: NO behaviour change visible to the user or the frontend (wording, JSON, requests sent, refusal order, progress phases); every existing test keeps passing unchanged unless it only moved; data-safety rules of AGENTS.md hold (placement, Trash, never overwrite, take-back); the guard tests (trash crate, std::fs::rename) stay green and their allowed lists follow moved code. Before committing: npx tsc --noEmit; npm run build; cd src-tauri && cargo test --locked (report counts per crate); npm test if any frontend file changed. Small commits where natural.\nReturn: commits (hash + subject), test counts, what changed, decisions taken, spec corrections, open problems.'

const IMPL = { type: 'object', properties: {
  commits: { type: 'array', items: { type: 'string' } }, tests: { type: 'string' }, changed: { type: 'string' },
  decisions: { type: 'string' }, spec_corrections: { type: 'string' }, problems: { type: 'string' } },
  required: ['commits', 'tests', 'changed', 'decisions', 'spec_corrections', 'problems'] }

phase('Implement')
const impl = await agent(RULES + '\n\n' + STEP, { label: 'implement step ' + args.step, phase: 'Implement', schema: IMPL })

phase('Review')
const FIND = { type: 'object', properties: { findings: { type: 'array', items: { type: 'object', properties: {
  file_line: { type: 'string' }, problem: { type: 'string' }, why_it_matters: { type: 'string' }, fix: { type: 'string' }, severity: { type: 'string', enum: ['blocker', 'major', 'minor'] } },
  required: ['file_line', 'problem', 'why_it_matters', 'fix', 'severity'] } } }, required: ['findings'] }
const REV = 'READ-ONLY review of branch ' + args.branch + ' in ' + WT + ' against origin/main (git -C ' + WT + ' diff origin/main...HEAD). Do not edit, commit or push. SPEED RULE: do NOT build or run cargo/npm at all; the implementer ran the full suites and the commit hook ran them again with coverage. Review by reading the diff and the code it touches; if a claim needs a test to settle, describe the test in your finding instead of running it. The step is MIGRATION STEP ' + args.step + ' of ' + SPEC + ':\n' + args.task + '\nThe implementer reported: ' + JSON.stringify(impl) + '\nReport only concrete problems with file:line. No style nits.'
const reviews = await parallel([
  () => agent(REV + '\n\nLens: BEHAVIOUR PRESERVATION AND DATA SAFETY. Trace every changed command/path against origin/main: same results, same wording and refusal order, same JSON, same requests, same progress; no path that used to refuse now runs (or the reverse); no fallthrough to a wrong provider; data-safety rules intact. Would any existing test have caught a regression here, and are new tests meaningful?', { label: 'review:behaviour', phase: 'Review', schema: FIND }),
  () => agent(REV + '\n\nLens: DESIGN FIT AND COMPLETENESS. Does the step do what the spec step says (nothing missing, nothing from later steps half-done), using the places interface as designed (Registry D10, Provider D3, Extensions D4, Capabilities D5, Transfer D6)? Dead code, duplicated logic left behind, AGENTS.md/spec drift, guard-test allowed lists, workspace members, Cargo.lock minimal.', { label: 'review:design', phase: 'Review', schema: FIND }),
])
const findings = reviews.filter(Boolean).flatMap((r) => r.findings || [])
log(findings.length + ' review findings')

phase('Fix')
let fix = null
if (findings.length) {
  fix = await agent(RULES_FAST + '\n\nYou are finishing MIGRATION STEP ' + args.step + ' (' + SPEC + ') on branch ' + args.branch + ' in ' + WT + ':\n' + args.task + '\nTwo reviewers reported the findings below. Verify each against the code; fix what is real (same rules), explain what is not. Put all fixes in ONE commit. Before committing run npx tsc --noEmit, npm run build, and the tests of the crates/files you touched (the final gate phase runs every test with coverage). npm test only for touched frontend test files. Report counts.\n\nFindings:\n' + JSON.stringify(findings, null, 1),
    { label: 'fix findings', phase: 'Fix', schema: { type: 'object', properties: {
      handled: { type: 'array', items: { type: 'object', properties: { finding: { type: 'string' }, outcome: { type: 'string', enum: ['fixed', 'not_real', 'deferred'] }, note: { type: 'string' } }, required: ['finding', 'outcome', 'note'] } },
      commits: { type: 'array', items: { type: 'string' } }, tests: { type: 'string' }, problems: { type: 'string' } }, required: ['handled', 'commits', 'tests', 'problems'] } })
}
if (fix && fix.commits && fix.commits.length) {
  phase('Recheck')
  const re = await agent(REV.split('The implementer reported:')[0] + '\nThis is a RECHECK of the fix commits only (' + fix.commits.join('; ') + '), made after the first reviews to answer: ' + JSON.stringify(findings.map(f => f.problem)).slice(0, 6000) + '\nLook for anything those commits broke or got wrong: data safety (AGENTS.md "Never lose data"), behaviour vs origin/main (wording, refusal order, JSON, requests), panics/unwraps on inputs a path can carry, docs drift. For each candidate, try hard to refute it yourself; report only what survives, with a concrete failing scenario.', { label: 'recheck fixes', phase: 'Recheck', schema: FIND })
  const more = (re && re.findings) || []
  log(more.length + ' recheck findings')
  if (more.length) {
    await agent(RULES_FAST + '\n\nFix these findings on branch ' + args.branch + ' in ' + WT + ' (MIGRATION STEP ' + args.step + '): verify each, fix what is real with a test, one commit with --no-verify, run the touched crates\' tests.\n' + JSON.stringify(more, null, 1), { label: 'fix recheck', phase: 'Recheck', schema: { type: 'object', properties: { commits: { type: 'array', items: { type: 'string' } }, notes: { type: 'string' } }, required: ['commits', 'notes'] } })
  }
}

phase('Gate')
const gate = await agent(RULES_FAST + '\n\nFINAL GATE for branch ' + args.branch + ' in ' + WT + ' (MIGRATION STEP ' + args.step + ', ' + SPEC + '), run once before the PR. The commits so far skipped the pre-commit hook on the owner\'s decision, so this is the full check. 1) git status must be clean; commit any leftover with --no-verify after judging it. 2) Free disk: cargo clean the shared target ' + MAIN + '/src-tauri/target if under 12 GB free. 3) npx tsc --noEmit; npm run build. 4) From ' + WT + ': node scripts/crap-gate.mjs --rust if no file under src/, tests/, dev/ or scripts/ changed versus origin/main, else node scripts/crap-gate.mjs (both halves). Give it a timeout of at least 45 minutes. It runs every test with coverage: report per-crate test counts and npm test counts from its output. 5) If any test or the gate fails, fix the cause (test or simplify; never touch .crap-baseline.json), commit with --no-verify, and rerun the gate until it passes. 6) At the end cargo clean --manifest-path ' + WT + '/src-tauri/Cargo.toml. Return pass/fail, counts, and any commits.',
  { label: 'final gate', phase: 'Gate', schema: { type: 'object', properties: { passed: { type: 'boolean' }, tests: { type: 'string' }, commits: { type: 'array', items: { type: 'string' } }, problems: { type: 'string' } }, required: ['passed', 'tests', 'commits', 'problems'] } })
return { impl, findings, fix, gate }

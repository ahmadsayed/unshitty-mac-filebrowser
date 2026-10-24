#!/usr/bin/env node
// CRAP gate: fail when a function's CRAP score is above the threshold and is
// new or worse than in `.crap-baseline.json`. Existing offenders may stay but
// may not grow; new code must stay under the threshold.
//
//   node scripts/crap-gate.mjs             check everything
//   node scripts/crap-gate.mjs --staged    check only languages with staged changes (pre-commit)
//   node scripts/crap-gate.mjs --update    rewrite the baseline from the current tree
//
// Rust is scored with real coverage (cargo llvm-cov + cargo-crap), so the Rust
// tests must pass. TypeScript has no tests yet, so fallow scores it from
// complexity alone (a function with cyclomatic complexity 6 already scores 42).

import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE = join(ROOT, ".crap-baseline.json");
const OUT = join(ROOT, "src-tauri", "target", "crap");
const THRESHOLD = 30;
// Scores are rounded to one decimal; coverage noise below this isn't a regression.
const TOLERANCE = 0.5;
const FALLOW = "fallow@3.35.0";

const args = new Set(process.argv.slice(2));
const update = args.has("--update");
const staged = args.has("--staged");

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function run(cmd, argv, opts = {}) {
  const result = spawnSync(cmd, argv, { cwd: ROOT, encoding: "utf8", maxBuffer: 256 << 20, ...opts });
  if (result.error) throw new Error(`${cmd}: ${result.error.message}`);
  return result;
}

function fail(message) {
  console.error(`\ncrap-gate: ${message}`);
  process.exit(1);
}

function have(cmd, argv) {
  return run(cmd, argv).status === 0;
}

function stagedFiles() {
  return execFileSync("git", ["diff", "--cached", "--name-only", "--diff-filter=ACMR"], { cwd: ROOT, encoding: "utf8" })
    .split("\n")
    .filter(Boolean);
}

/**
 * Key every function by file + name; functions sharing a name in one file
 * (closures, `<arrow>`) get `#2`, `#3` in line order. Lines aren't part of the
 * key, so editing code above a function doesn't make it look new.
 */
function keyed(rows) {
  rows.sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line);
  const seen = new Map();
  const out = {};
  for (const row of rows) {
    const base = `${row.file}::${row.name}`;
    const n = (seen.get(base) ?? 0) + 1;
    seen.set(base, n);
    out[n === 1 ? base : `${base}#${n}`] = { score: Math.round(row.crap * 10) / 10, line: row.line };
  }
  return out;
}

// ---------------------------------------------------------------------------
// Analyses
// ---------------------------------------------------------------------------

function analyzeRust() {
  for (const [cmd, install] of [
    ["cargo-llvm-cov", "cargo install --locked cargo-llvm-cov && rustup component add llvm-tools-preview"],
    ["cargo-crap", "cargo install --locked cargo-crap"],
  ]) {
    if (!have("cargo", [cmd.replace("cargo-", ""), "--version"])) fail(`${cmd} is missing. Install it with:\n  ${install}`);
  }
  // tauri's generate_context! reads ../dist.
  if (!existsSync(join(ROOT, "dist", "index.html"))) {
    console.log("crap-gate: building the frontend first (dist/ is missing)…");
    if (run("npm", ["run", "build"], { stdio: "inherit" }).status !== 0) fail("npm run build failed");
  }
  mkdirSync(OUT, { recursive: true });
  const lcov = join(OUT, "lcov.info");
  const json = join(OUT, "rust.json");
  console.log("crap-gate: Rust tests with coverage…");
  const cov = run("cargo", ["llvm-cov", "--lcov", "--output-path", lcov], { cwd: join(ROOT, "src-tauri") });
  if (cov.status !== 0) {
    console.error(cov.stdout.split("\n").filter((l) => /FAILED|panicked|error/.test(l)).join("\n"));
    fail("the Rust tests failed, so there is no coverage to score");
  }
  const crap = run("cargo", ["crap", "--lcov", lcov, "--format", "json", "--output", json], { cwd: join(ROOT, "src-tauri") });
  if (!existsSync(json)) fail(`cargo crap failed:\n${crap.stderr}`);
  const entries = JSON.parse(readFileSync(json, "utf8")).entries;
  return keyed(
    entries.map((e) => ({ file: `src-tauri/${e.file.replace(/^\.\//, "")}`, name: e.function, line: e.line, crap: e.crap })),
  );
}

function analyzeTs() {
  console.log("crap-gate: TypeScript complexity…");
  const result = run("npx", ["-y", FALLOW, "health", "--complexity", "--report-only", "--max-crap", String(THRESHOLD), "--format", "json"]);
  let report;
  try {
    report = JSON.parse(result.stdout);
  } catch {
    fail(`fallow failed:\n${result.stderr || result.stdout}`);
  }
  // Findings are the functions at or over a complexity or CRAP limit; anything
  // under the CRAP threshold is dropped below, so that is all the gate needs.
  return keyed(report.findings.map((f) => ({ file: f.path, name: f.name, line: f.line, crap: f.crap })));
}

// ---------------------------------------------------------------------------
// Gate
// ---------------------------------------------------------------------------

const only = (scores) => Object.fromEntries(Object.entries(scores).filter(([, v]) => v.score > THRESHOLD));

let langs = ["rust", "ts"];
if (staged) {
  const files = stagedFiles();
  langs = [];
  if (files.some((f) => f.startsWith("src-tauri/") && /\.(rs|toml|lock)$/.test(f))) langs.push("rust");
  if (files.some((f) => /^(src|dev)\/.*\.ts$/.test(f) || f === "package.json")) langs.push("ts");
  if (!langs.length) process.exit(0);
}

const baseline = existsSync(BASELINE) ? JSON.parse(readFileSync(BASELINE, "utf8")) : { threshold: THRESHOLD, rust: {}, ts: {} };
const current = {};
if (langs.includes("rust")) current.rust = only(analyzeRust());
if (langs.includes("ts")) current.ts = only(analyzeTs());

if (update) {
  const next = { threshold: THRESHOLD, rust: current.rust ?? baseline.rust, ts: current.ts ?? baseline.ts };
  for (const lang of ["rust", "ts"]) {
    next[lang] = Object.fromEntries(Object.entries(next[lang]).sort().map(([k, v]) => [k, v.score ?? v]));
  }
  writeFileSync(BASELINE, JSON.stringify(next, null, 2) + "\n");
  console.log(`crap-gate: baseline written (${Object.keys(next.rust).length} Rust, ${Object.keys(next.ts).length} TypeScript functions above ${THRESHOLD}).`);
  process.exit(0);
}

const problems = [];
const improved = [];
for (const lang of langs) {
  const known = baseline[lang] ?? {};
  for (const [key, { score, line }] of Object.entries(current[lang])) {
    const before = known[key];
    const [file, name] = key.split("::");
    const where = `${file}:${line} ${name}`;
    if (before === undefined) problems.push(`  ${score.toFixed(1).padStart(8)}  new        ${where}`);
    else if (score > before + TOLERANCE) problems.push(`  ${score.toFixed(1).padStart(8)}  was ${String(before).padEnd(6)} ${where}`);
  }
  for (const [key, before] of Object.entries(known)) {
    const now = current[lang][key]?.score;
    if (now === undefined || now < before - TOLERANCE) improved.push(key);
  }
}

if (improved.length) {
  console.log(`crap-gate: ${improved.length} function(s) improved or dropped under ${THRESHOLD}; run \`npm run crap:baseline\` to lock that in.`);
}
if (problems.length) {
  console.error(`\ncrap-gate: CRAP above ${THRESHOLD} that is new or worse than the baseline:\n`);
  console.error(problems.join("\n"));
  fail(
    "simplify these functions or cover them with tests. If the score is " +
      "accepted on purpose, run `npm run crap:baseline` and commit .crap-baseline.json.",
  );
}
console.log(`crap-gate: ok (${langs.join(", ")}).`);

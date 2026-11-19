// Test command for the QA mutation check — `.claude/skill-bindings/qa-mutate.json` names this file, and the shared
// engram-qa `mutate.mjs` spawns it (no shell) after breaking one line, then restores the line. qa binding: mutation check.
//   node scripts/qa-mutate-test.mjs <pkg> <name>
// <pkg>:
//   frontend                  <name> = one vitest file, repo-relative (src/.../x.test.ts) — the whole file runs
//   <cargo package>           every default test target of the package
//   <cargo package>/lib       the lib unit tests (--lib)
//   <cargo package>/bins      the bin unit tests (--bins)
//   <cargo package>/<target>  one integration test target (--test <target>)
// Rust <name> = the full test path inside its target (writable::tests::foo). It runs with --exact --include-ignored, so an
//   #[ignore] test runs only when it is the one named.
// The command builds (cargo test compiles the broken line; vitest transforms on load) in the tree's own target/ — in an
//   isolated copy, the copy's target/. No --target-dir: mutate.mjs passes only <pkg> and <name>.
// The shell package rebuilds the daemon exe first: its tests launch <target>/debug/engram-dashboard-daemon.exe, and cargo
//   does not rebuild another package's binary for a test.
// Exit: 0 = the named test ran and passed · 1 = it ran and failed · 2 = bad arguments · 3 = no test ran (wrong name or
//   target) · 4 = build failure or no test result. The last line is the summary: "qa-mutate-test: <outcome> ran=.. passed=.. failed=..".
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SHELL_PKG = 'engram-dashboard';
const DAEMON_PKG = 'engram-dashboard-daemon';
const MAX_BUFFER = 256 * 1024 * 1024;

function finish(code, outcome, counts) {
	const c = counts ? ` ran=${counts.passed + counts.failed} passed=${counts.passed} failed=${counts.failed}` : '';
	console.log(`qa-mutate-test: ${outcome}${c}`);
	process.exit(code);
}

const usage = (why) => {
	console.error(`qa-mutate-test: ${why}`);
	console.error('usage: node scripts/qa-mutate-test.mjs <frontend | cargo package[/lib|/bins|/<test target>]> <test name or vitest file>');
	finish(2, 'bad-arguments');
};

// Child stdout is captured to count results and then echoed; stderr (cargo progress, compile errors) goes straight through.
function run(cmd, args) {
	console.log(`qa-mutate-test: $ ${[cmd, ...args].join(' ')}`);
	const r = spawnSync(cmd, args, { cwd: ROOT, stdio: ['ignore', 'pipe', 'inherit'], encoding: 'utf8', maxBuffer: MAX_BUFFER, windowsHide: true });
	if (r.stdout) process.stdout.write(r.stdout);
	if (r.error) {
		console.error(`qa-mutate-test: could not start ${cmd}: ${r.error.message}`);
		finish(4, 'error');
	}
	return { status: r.status, out: (r.stdout || '').replace(/\x1b\[[0-9;]*m/g, '') };
}

function judge(status, counts) {
	const ran = counts.passed + counts.failed;
	if (counts.failed > 0) finish(1, 'test-failed', counts);
	if (status !== 0) finish(4, 'build-or-run-error', counts);
	if (ran === 0) finish(3, 'no-test-ran', counts);
	finish(0, 'test-passed', counts);
}

function runVitest(file) {
	if (!/\.test\.tsx?$/.test(file) || !fs.existsSync(path.join(ROOT, file))) usage(`not a vitest file in this tree: ${file}`);
	const vitest = path.join(ROOT, 'node_modules', 'vitest', 'vitest.mjs');
	if (!fs.existsSync(vitest)) usage('node_modules/vitest is missing — run npm ci first');
	const { status, out } = run(process.execPath, [vitest, 'run', file]);
	const line = out.split('\n').find((l) => /^\s*Tests\s/.test(l)) || '';
	const num = (word) => Number((line.match(new RegExp(`(\\d+) ${word}`)) || [0, 0])[1]);
	judge(status, { passed: num('passed'), failed: num('failed') });
}

function runCargo(spec, name) {
	const [pkg, target, extra] = spec.split('/');
	if (!pkg || extra !== undefined || target === '') usage(`bad cargo spec: ${spec}`);
	const select = target === undefined ? [] : target === 'lib' ? ['--lib'] : target === 'bins' ? ['--bins'] : ['--test', target];
	if (pkg === SHELL_PKG) {
		// Cargo.toml turns the built-in lib test target off; picking it with --lib builds an exe that dies at load.
		if (target === undefined || target === 'lib') usage(`${SHELL_PKG} needs a test target (lib unit tests = ${SHELL_PKG}/lib_unit)`);
		const pre = run('cargo', ['build', '-p', DAEMON_PKG]);
		if (pre.status !== 0) finish(4, 'build-or-run-error');
	}
	const { status, out } = run('cargo', ['test', '-p', pkg, ...select, '--', '--exact', name, '--include-ignored', '--test-threads=1']);
	const counts = { passed: 0, failed: 0 };
	for (const m of out.matchAll(/test result: \w+\. (\d+) passed; (\d+) failed/g)) {
		counts.passed += Number(m[1]);
		counts.failed += Number(m[2]);
	}
	judge(status, counts);
}

const [spec, name, ...rest] = process.argv.slice(2);
if (!spec || !name || rest.length) usage('expected exactly two arguments');
if (spec === 'frontend') runVitest(name);
else runCargo(spec, name);

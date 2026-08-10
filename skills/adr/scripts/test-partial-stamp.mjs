// adr.mjs partial supersede "상태줄 부분폐기 도장" 하네스.
//   node 내장만(외부 의존 0). 쓰기는 os.tmpdir() 아래 매 실행 고유 픽스처 폴더에서만 한다.
//
// 실행:
//   node <경로>/test-partial-stamp.mjs
//   ADR_REGRESSION_DIR=<프로젝트 루트> node <경로>/test-partial-stamp.mjs   # 실데이터 read-only 회귀 추가
//     ADR_REGRESSION_EXPECT='{"adrCount":106,"lintAdvisories":5,"indexDiffs":18,"indexWarnings":32,"indexTruncationWarnings":13}'
//     를 같이 주면 베이스라인 수치까지 대조한다(생략하면 오류 0·무변경만 검사하고 수치는 기록만).
//     ★주의: lintAdvisories 는 *프로젝트 루트*를 가리킬 때의 값이다 — 코드 앵커 스캔(crates/ src/ …) 결과가 섞인다.
//     docs/decisions 만 복사한 사본을 가리키면 앵커가 없어 권고 수가 줄어든다(대시보드 기준 5 → 1).
//     사본으로 돌릴 땐 lintAdvisories 를 빼거나 그 환경의 실측치로 바꾼다.
//   회귀 대상은 읽기만 한다 — lint / index --check 만 돌리고 폴더 해시로 무변경을 증명한다.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SCRIPT = path.join(HERE, 'adr.mjs');

// 순수 함수 직접 호출용 import(ADR_LIB_ONLY=1 이면 adr.mjs 가 CLI를 안 돈다).
//   import 직후 지운다 — 자식 프로세스(CLI 테스트)에 새면 CLI가 no-op 된다.
process.env.ADR_LIB_ONLY = '1';
const lib = await import(pathToFileURL(SCRIPT).href);
delete process.env.ADR_LIB_ONLY;

const TMP = fs.mkdtempSync(path.join(os.tmpdir(), `adr-stamp-test-${process.pid}-`));

// ── 러너 ──────────────────────────────────────────────────────────────────────
const results = [];
function test(name, fn) {
  try { fn(); results.push({ name, ok: true }); console.log(`PASS  ${name}`); }
  catch (e) { results.push({ name, ok: false, err: e.message }); console.log(`FAIL  ${name}\n      ${e.message}`); }
}
function skip(name, why) { results.push({ name, ok: true, skipped: true }); console.log(`SKIP  ${name} — ${why}`); }

// envOverride: 키 값이 undefined 면 그 키를 지운다(미설정 상태 재현).
function run(args, cwd, envOverride = {}) {
  const env = { ...process.env, ADR_LIB_ONLY: '0' }; // 자식은 항상 CLI로 돌아야 한다(테스트가 명시 지정하면 그걸 따름)
  for (const [k, v] of Object.entries(envOverride)) { if (v === undefined) delete env[k]; else env[k] = v; }
  const r = spawnSync(process.execPath, [SCRIPT, ...args], { encoding: 'utf8', cwd, env });
  let json = null;
  try { json = JSON.parse(r.stdout); } catch { /* 파싱 실패 = 호출부가 raw 로 본다 */ }
  return { code: r.status, json, stdout: r.stdout, stderr: r.stderr };
}

const read = (f) => fs.readFileSync(f, 'utf8');
const statusLineOf = (f) => read(f).split(/\r?\n/).find((l) => /^-\s*상태:/.test(l)) ?? null;
const metaLineOf = (f) => read(f).split(/\r?\n/).find((l) => /^-\s/.test(l) && /상태:/.test(l)) ?? null;
const relatedLineOf = (f) => read(f).split(/\r?\n/).find((l) => /^-\s*관련:/.test(l)) ?? null;
const countOf = (hay, needle) => hay.split(needle).length - 1;
const inDir = (d, num) => {
  const hit = fs.readdirSync(d).find((n) => n.startsWith(`${String(num).padStart(4, '0')}-`) && n.endsWith('.md'));
  return hit ? path.join(d, hit) : null;
};

// ── 픽스처 ────────────────────────────────────────────────────────────────────
const DASH_ADR = (num, title, status = '확정 (2026-07-01, 근거: TODO)', related = 'CLAUDE.md §1') =>
  `# ADR-${String(num).padStart(4, '0')}: ${title}

- 상태: ${status}
- 관련: ${related}

## 맥락
TODO

## 결정
TODO
`;
// factory 경량 포맷(references/formats/adr-light.template.md) — 결합 메타 줄.
const LIGHT_ADR = (num, title, withRelated) => `# ADR-${String(num).padStart(4, '0')}: ${title}

- 날짜: 2026-07-01 · 상태: 확정 · 결정자: 사용자
${withRelated ? '- 관련: CLAUDE.md §1\n' : ''}
## 결정

TODO
`;
const README = `# 결정 기록

## 인덱스

| # | 제목 | 상태 |
|---|---|---|

## 템플릿

TODO
`;

function mkFixture(name, files, withReadme = true) {
  const d = path.join(TMP, name);
  fs.rmSync(d, { recursive: true, force: true });
  fs.mkdirSync(d, { recursive: true });
  for (const [f, body] of Object.entries(files)) fs.writeFileSync(path.join(d, f), body, 'utf8');
  if (withReadme) fs.writeFileSync(path.join(d, 'README.md'), README, 'utf8');
  return d;
}

// ── 케이스 1~5: dashboard 스타일 픽스처 A ─────────────────────────────────────
const A = mkFixture('A', { '0001-첫-결정.md': DASH_ADR(1, '첫 결정') });
const A_OLD = path.join(A, '0001-첫-결정.md');
const A_FLAGS = ['--anchor-roots', '', '--dir', A];

test('1. partial: 상태줄 도장(head 끝) + 어휘 보존 + 관련 양방향 + 새 ADR 생성', () => {
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 A', '--title', '새 결정 A', '--dir', A]);
  assert.equal(r.code, 0, `exit!=0: ${r.stdout}`);
  assert.equal(r.json.newNum, 2);
  assert.deepEqual(r.json.statusStamp, { stamped: true, position: 'line-end', reason: null });

  const sl = statusLineOf(A_OLD);
  assert.equal(sl, '- 상태: 확정 (2026-07-01, 근거: TODO) · 부분 폐기 by ADR-0002 (조항 A)');
  assert.match(sl, /^- 상태: 확정(?![가-힣])/);                       // 어휘가 줄 선두에 그대로
  assert.match(relatedLineOf(A_OLD), /· Amended by ADR-0002 \(조항 A\)$/);

  const nf = inDir(A, 2);
  assert.ok(nf, '새 ADR-0002 파일 없음');
  assert.match(relatedLineOf(nf), /^- 관련: Amends ADR-0001 \(조항 A\)/);
  assert.match(statusLineOf(nf), /^- 상태: 확정 \(/);                  // 새 ADR 은 도장 없음
});

test('2. 다른 번호의 두 번째 partial → 절이 하나 더(번호 오름차순)', () => {
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 B', '--title', '새 결정 B', '--dir', A]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(r.json.newNum, 3);
  assert.equal(r.json.statusStamp.stamped, true);

  const sl = statusLineOf(A_OLD);
  assert.ok(sl.includes('· 부분 폐기 by ADR-0002 (조항 A)'), sl);
  assert.ok(sl.includes('· 부분 폐기 by ADR-0003 (조항 B)'), sl);
  assert.ok(sl.indexOf('ADR-0002') < sl.indexOf('ADR-0003'), `오름차순 아님: ${sl}`);
  assert.match(sl, /^- 상태: 확정 \(2026-07-01, 근거: TODO\) ·/);      // 어휘·근거 괄호 원형 보존
  assert.match(relatedLineOf(A_OLD), /Amended by ADR-0003 \(조항 B\)$/);
});

test('3. 멱등(CLI): 같은 새 ADR 번호 도장이 이미 있으면 재삽입 안 함', () => {
  // 정직 메모: supersede 는 매번 새 번호를 발급하므로 "같은 명령 재실행"으로는 이 상태에 못 간다.
  //   다음 발급 번호(0004)의 도장을 손으로 미리 박아 재도장 경로를 강제 재현한다(함수 레벨 멱등은 3b).
  fs.writeFileSync(A_OLD, read(A_OLD).replace(/^(- 상태: .*)$/m, '$1 · 부분 폐기 by ADR-0004 (조항 C)'), 'utf8');
  assert.equal(countOf(statusLineOf(A_OLD), '부분 폐기 by ADR-0004'), 1);

  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 C', '--title', '새 결정 C', '--dir', A]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(r.json.newNum, 4);
  assert.deepEqual(r.json.statusStamp, { stamped: false, position: null, reason: 'already-stamped' });

  const sl = statusLineOf(A_OLD);
  assert.equal(countOf(sl, '부분 폐기 by ADR-0004'), 1, `중복 도장: ${sl}`);
  assert.equal(countOf(sl, '부분 폐기 by ADR-0002'), 1, sl);
  assert.equal(countOf(sl, '부분 폐기 by ADR-0003'), 1, sl);
  assert.match(relatedLineOf(A_OLD), /Amended by ADR-0004 \(조항 C\)$/); // 관련줄 링크는 여전히 박힌다
});

test('3b. 멱등(함수 직접 호출): 같은 번호 2회 = 무변경, 다른 번호 = 오름차순 추가', () => {
  const base = '- 상태: 확정 (2026-07-01, 근거: TODO)';
  const once = lib.stampPartialStatusLine(base, 3, '조항 Z');
  assert.equal(once.stamped, true);
  assert.equal(once.line, `${base} · 부분 폐기 by ADR-0003 (조항 Z)`);

  const twice = lib.stampPartialStatusLine(once.line, 3, '조항 Z');
  assert.equal(twice.stamped, false);
  assert.equal(twice.reason, 'already-stamped');
  assert.equal(twice.line, once.line, '재호출에서 줄이 변형됨');

  const other = lib.stampPartialStatusLine(once.line, 4, '조항 W');
  assert.equal(other.stamped, true);
  assert.ok(other.line.indexOf('ADR-0003') < other.line.indexOf('ADR-0004'), other.line);
  assert.equal(lib.stampPartialStatusLine(other.line, 4, '조항 W').stamped, false);

  // 단서절 있는 줄: 도장은 괄호 밖 첫 em-dash 앞(= head 끝), 괄호 안 em-dash 는 경계 아님.
  const dashed = lib.stampPartialStatusLine('- 상태: 확정 (근거: 사용자 결정 — 리뷰) — 단, X 재검토', 5, '조항 Y');
  assert.equal(dashed.position, 'before-em-dash');
  assert.equal(dashed.line, '- 상태: 확정 (근거: 사용자 결정 — 리뷰) · 부분 폐기 by ADR-0005 (조항 Y) — 단, X 재검토');
});

test('4. lint(픽스처 A): 오류 0 — 도장 후에도 상태 어휘 정상 인식', () => {
  const r = run(['lint', ...A_FLAGS]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(r.json.errorCount, 0, JSON.stringify(r.json.findings));
  assert.equal(r.json.clean, true);
  assert.equal(r.json.count, 4);
  assert.equal(r.json.findings.filter((f) => f.kind === 'invalid-status-vocab').length, 0);
  assert.equal(r.json.findings.filter((f) => f.kind === 'amend-unidirectional').length, 0);
});

test('5. index --write 2회: 멱등 + 도장 생존 + 셀 파생 정상', () => {
  const r1 = run(['index', '--write', ...A_FLAGS]);
  assert.equal(r1.code, 0, r1.stdout);
  assert.equal(r1.json.changed, true);
  const after1 = read(path.join(A, 'README.md'));
  const body1 = read(A_OLD);

  const r2 = run(['index', '--write', ...A_FLAGS]);
  assert.equal(r2.json.changed, false, '2회차에서 변경 발생 = 비멱등');
  assert.equal(read(path.join(A, 'README.md')), after1, 'README 2회차 내용 불일치 = 비멱등');
  assert.equal(read(A_OLD), body1, 'index --write 가 본문을 건드림');

  assert.ok(statusLineOf(A_OLD).includes('· 부분 폐기 by ADR-0002 (조항 A)'), '도장 소실');
  const row = after1.split(/\r?\n/).find((l) => l.startsWith('| [0001]'));
  assert.ok(row && row.includes('확정 (부분 폐기 by ADR-0002: 조항 A)'), `셀 파생 이상: ${row}`);
  assert.equal(run(['index', '--check', ...A_FLAGS]).json.clean, true);
});

// ── 케이스 6: full 모드 불변 ──────────────────────────────────────────────────
test('6. full 모드 종전 동작 유지(폐기 + 취소선, 도장 없음)', () => {
  const B = mkFixture('B', { '0001-옛-결정.md': DASH_ADR(1, '옛 결정') });
  const oldF = path.join(B, '0001-옛-결정.md');
  const r = run(['supersede', '--old', '1', '--mode', 'full', '--title', '전체 대체', '--dir', B]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(r.json.statusStamp, undefined, 'full 에 statusStamp 필드가 붙음(경로 오염)');
  assert.equal(
    statusLineOf(oldF),
    '- 상태: **폐기 (Superseded by ADR-0002)** — TODO 사유. ~~확정 (2026-07-01, 근거: TODO)~~',
  );
  assert.ok(!read(oldF).includes('부분 폐기'), '부분폐기 도장이 full 에 섞임');
  assert.match(relatedLineOf(inDir(B, 2)), /^- 관련: Supersedes ADR-0001/);
  assert.equal(run(['lint', '--anchor-roots', '', '--dir', B]).json.errorCount, 0);
});

// ── 케이스 8: factory 경량(결합 메타 줄) 가드 ─────────────────────────────────
test('8a. 경량(관련줄 없음): partial 은 종전대로 거부 + 파일 무변경', () => {
  const C = mkFixture('C1', { '0001-경량-결정.md': LIGHT_ADR(1, '경량 결정', false) });
  const f = path.join(C, '0001-경량-결정.md');
  const before = read(f);
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 X', '--title', '새 경량', '--dir', C]);
  assert.equal(r.code, 1, r.stdout);
  assert.match(r.json.error, /"- 관련:" 줄이 없어/);
  assert.equal(read(f), before, '거부인데 옛 파일이 변형됨');
  assert.equal(inDir(C, 2), null, '거부인데 새 파일이 생성됨');
});

test('8b. 경량(관련줄 있음): 관련 링크만 박고 결합 메타 줄엔 도장 안 찍음', () => {
  const C = mkFixture('C2', { '0001-경량-결정.md': LIGHT_ADR(1, '경량 결정', true) });
  const f = path.join(C, '0001-경량-결정.md');
  const metaBefore = metaLineOf(f);
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 X', '--title', '새 경량', '--dir', C]);
  assert.equal(r.code, 0, r.stdout);
  assert.deepEqual(r.json.statusStamp, { stamped: false, position: null, reason: 'combined-meta-line' });
  assert.equal(metaLineOf(f), metaBefore, '결합 메타 줄이 변형됨');
  assert.match(relatedLineOf(f), /· Amended by ADR-0002 \(조항 X\)$/);
});

test('8c. 경량: full 은 종전대로 결합 메타 줄 거부', () => {
  const C = mkFixture('C3', { '0001-경량-결정.md': LIGHT_ADR(1, '경량 결정', true) });
  const f = path.join(C, '0001-경량-결정.md');
  const before = read(f);
  const r = run(['supersede', '--old', '1', '--mode', 'full', '--title', '새 경량', '--dir', C]);
  assert.equal(r.code, 1, r.stdout);
  assert.match(r.json.error, /결합 메타 줄/);
  assert.equal(read(f), before);
});

// ── 케이스 9~11: 도장 위치가 기존 가드를 안 깨는지 ────────────────────────────
test('9a. em-dash 단서절 상태줄: 도장은 단서절 앞(head 끝), 단서절 보존, lint 0', () => {
  const D = mkFixture('D1', {
    '0001-단서절-결정.md': DASH_ADR(1, '단서절 결정', '확정 (2026-07-01, 근거: TODO) — 단, C3 는 재검토 대상'),
  });
  const f = path.join(D, '0001-단서절-결정.md');
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 D', '--title', '단서절 개정', '--dir', D]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(r.json.statusStamp.position, 'before-em-dash');
  assert.equal(
    statusLineOf(f),
    '- 상태: 확정 (2026-07-01, 근거: TODO) · 부분 폐기 by ADR-0002 (조항 D) — 단, C3 는 재검토 대상',
  );
  assert.equal(run(['lint', '--anchor-roots', '', '--dir', D]).json.errorCount, 0);
});

test('9b. full→partial: 전체폐기 줄에 도장해도 "폐기 (Superseded by …)" 연속성·가드 유지', () => {
  const E = mkFixture('E1', { '0001-옛-결정.md': DASH_ADR(1, '옛 결정') });
  const f = path.join(E, '0001-옛-결정.md');
  assert.equal(run(['supersede', '--old', '1', '--mode', 'full', '--title', '전체 대체', '--dir', E]).code, 0);
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 E', '--title', '부분 개정', '--dir', E]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(r.json.statusStamp.position, 'before-em-dash');
  const sl = statusLineOf(f);
  assert.match(sl, /\*\*폐기 \(Superseded by ADR-0002\)\*\* · 부분 폐기 by ADR-0003 \(조항 E\) —/);
  // 전체폐기 재래핑 가드가 여전히 잡아야 한다(도장이 "폐기"와 "(Superseded by" 를 쪼개면 뚫린다).
  const again = run(['supersede', '--old', '1', '--mode', 'full', '--title', '또 대체', '--dir', E]);
  assert.equal(again.code, 1, `가드 뚫림: ${again.stdout}`);
  assert.match(again.json.error, /이미 전체폐기됨/);
  assert.equal(run(['lint', '--anchor-roots', '', '--dir', E]).json.errorCount, 0);
});

test('10. 비-굵은 "폐기 (Superseded by ADR-N) — 사유" 실데이터 형태: 가드·인덱스 파생 유지', () => {
  const F = mkFixture('F1', {
    '0001-옛-결정.md': DASH_ADR(1, '옛 결정', '폐기 (Superseded by ADR-0002) — 원안 기록은 이력으로 보존'),
    '0002-대체-결정.md': DASH_ADR(2, '대체 결정', '확정 (2026-07-02, 근거: TODO)', 'Supersedes ADR-0001 · CLAUDE.md §1'),
  });
  const f = path.join(F, '0001-옛-결정.md');
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 F', '--title', '부분 개정', '--dir', F]);
  assert.equal(r.code, 0, r.stdout);
  const sl = statusLineOf(f);
  assert.equal(sl, '- 상태: 폐기 (Superseded by ADR-0002) · 부분 폐기 by ADR-0003 (조항 F) — 원안 기록은 이력으로 보존');
  assert.match(sl, /폐기 \(Superseded by ADR-0002\)/, '어휘와 전체폐기 링크가 쪼개짐');

  // ① full 재폐기 가드 유지 ② lint 오류 0 ③ 인덱스 셀이 전체폐기 포인터를 잃지 않음
  const again = run(['supersede', '--old', '1', '--mode', 'full', '--title', '또 대체', '--dir', F]);
  assert.equal(again.code, 1, `가드 뚫림: ${again.stdout}`);
  assert.match(again.json.error, /이미 전체폐기됨/);

  const l = run(['lint', '--anchor-roots', '', '--dir', F]);
  assert.equal(l.json.errorCount, 0, JSON.stringify(l.json.findings));

  run(['index', '--write', '--anchor-roots', '', '--dir', F]);
  const row = read(path.join(F, 'README.md')).split(/\r?\n/).find((x) => x.startsWith('| [0001]'));
  assert.ok(row.includes('폐기 (Superseded by ADR-0002)'), `인덱스 셀에서 전체폐기 포인터 소실: ${row}`);
  assert.equal(run(['index', '--check', '--anchor-roots', '', '--dir', F]).json.clean, true);
});

test('11. partial→full 순서: full 이 정상 동작 + 도장은 취소선 안에 이력 보존 + lint 0', () => {
  const G = mkFixture('G1', { '0001-옛-결정.md': DASH_ADR(1, '옛 결정') });
  const f = path.join(G, '0001-옛-결정.md');
  assert.equal(run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 G', '--title', '부분 개정', '--dir', G]).code, 0);
  const r = run(['supersede', '--old', '1', '--mode', 'full', '--title', '전체 대체', '--dir', G]);
  assert.equal(r.code, 0, `partial 뒤 full 이 거부됨: ${r.stdout}`);
  assert.equal(
    statusLineOf(f),
    '- 상태: **폐기 (Superseded by ADR-0003)** — TODO 사유. ~~확정 (2026-07-01, 근거: TODO) · 부분 폐기 by ADR-0002 (조항 G)~~',
  );
  const l = run(['lint', '--anchor-roots', '', '--dir', G]);
  assert.equal(l.json.errorCount, 0, JSON.stringify(l.json.findings));
});

// ── 케이스 12: clause 위생(파서 토큰 위조 차단) ───────────────────────────────
test('12. clause 위생: 파서 토큰·괄호 불균형 거부 + 파일 무변경', () => {
  const H = mkFixture('H1', { '0001-옛-결정.md': DASH_ADR(1, '옛 결정') });
  const f = path.join(H, '0001-옛-결정.md');
  const before = read(f);
  const bad = [
    ['조항 A · 조항 B', /가운뎃점/],
    ['조항 — 부연 설명', /em-dash/],
    ['조항 A · 부분 폐기 by ADR-0003 (가짜)', /가운뎃점|부분 폐기 by/],   // 멱등 검사 오탐 유도(M1)
    ['조항 (Superseded by ADR-9999)', /Superseded by/],                   // full 가드 위조
    ['조항 Amends ADR-0009', /Amends/],                                   // 관련줄 링크 위조
    ['조항 (미완', /괄호가 안 맞음/],
  ];
  for (const [clause, reMsg] of bad) {
    const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', clause, '--title', '개정', '--dir', H]);
    assert.equal(r.code, 1, `거부돼야 함: "${clause}" → ${r.stdout}`);
    assert.equal(r.json.ok, false);
    assert.match(r.json.error, reMsg, `에러 메시지 부적절: ${r.json.error}`);
    assert.match(r.json.error, /재시도/, '조치 안내(재시도) 없음');
    assert.equal(read(f), before, `거부인데 옛 파일 변형: "${clause}"`);
    assert.equal(inDir(H, 2), null, `거부인데 새 파일 생성: "${clause}"`);
  }
  // 정상 조항(괄호 포함)은 통과해야 한다 — 과잉 거부 방지.
  //   단, 괄호 든 조항은 인덱스 셀 파생에서 잘린다(선존 결함) — 그건 케이스 16이 경고로 확인한다.
  //   여기서 "통과"는 입력 위생 검사 통과일 뿐, 인덱스 셀이 온전하다는 뜻이 아니다.
  const ok = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '결정 2(배치) → 우클릭 메뉴', '--title', '개정', '--dir', H]);
  assert.equal(ok.code, 0, `정상 조항이 거부됨: ${ok.stdout}`);
});

// ── 케이스 13~16: 라운드2 리뷰 지적(F1 tail 스푸핑 · F2 lib 모드 · F3 전각 괄호 · F4 잘림 경고) ──
test('13. F1 멱등 스푸핑: 단서절(tail)의 가짜 도장은 진짜 도장을 막지 못함', () => {
  const I = mkFixture('I1', {
    '0001-옛-결정.md': DASH_ADR(1, '옛 결정', '확정 — 참고: · 부분 폐기 by ADR-0002 (가짜)'),
  });
  const f = path.join(I, '0001-옛-결정.md');
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 T', '--title', '개정', '--dir', I]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(r.json.statusStamp.stamped, true, 'tail 스푸핑에 속아 진짜 도장을 스킵함');
  const sl = statusLineOf(f);
  assert.equal(sl, '- 상태: 확정 · 부분 폐기 by ADR-0002 (조항 T) — 참고: · 부분 폐기 by ADR-0002 (가짜)');
  assert.equal(countOf(sl.split('—')[0], '부분 폐기 by ADR-0002'), 1, `head 에 도장 중복: ${sl}`);
  assert.equal(run(['lint', '--anchor-roots', '', '--dir', I]).json.errorCount, 0);

  // 함수 레벨: head 안 깊이 0의 같은 번호 도장(손으로 찍었어도 형식이 같으면 진짜)은 여전히 스킵.
  assert.equal(lib.stampPartialStatusLine('- 상태: 확정 · 부분 폐기 by ADR-0003 (조항) — 사유', 3, 'x').stamped, false);
  assert.equal(lib.stampPartialStatusLine('- 상태: 확정 — 참고: · 부분 폐기 by ADR-0003 (가짜)', 3, 'x').stamped, true);
});

test('17. G1 괄호 안 스푸핑: head 라도 괄호에 싸인 가짜 도장은 도장이 아님', () => {
  const N = mkFixture('N1', {
    '0001-옛-결정.md': DASH_ADR(1, '옛 결정', '확정 (참고: · 부분 폐기 by ADR-0002 (가짜)) — 단서'),
  });
  const f = path.join(N, '0001-옛-결정.md');
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 Q', '--title', '개정', '--dir', N]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(r.json.statusStamp.stamped, true, '괄호 안 인용에 속아 진짜 도장을 스킵함');
  assert.equal(
    statusLineOf(f),
    '- 상태: 확정 (참고: · 부분 폐기 by ADR-0002 (가짜)) · 부분 폐기 by ADR-0002 (조항 Q) — 단서',
  );
  assert.match(relatedLineOf(f), /Amended by ADR-0002 \(조항 Q\)$/); // 링크·도장이 함께 박힌다(반쪽 상태 아님)
  assert.equal(run(['lint', '--anchor-roots', '', '--dir', N]).json.errorCount, 0);

  // 함수 레벨: 깊이 0 진짜 도장은 스킵, 괄호 안 인용은 통과(중첩 괄호 포함).
  assert.equal(lib.stampPartialStatusLine('- 상태: 확정 (참고: · 부분 폐기 by ADR-0003 (가짜)) — 사유', 3, 'x').stamped, true);
  assert.equal(lib.stampPartialStatusLine('- 상태: 확정 · 부분 폐기 by ADR-0003 (조항 (중첩)) — 사유', 3, 'x').stamped, false);
  assert.equal(lib.stampPartialStatusLine('- 상태: 확정 （인용: · 부분 폐기 by ADR-0003 (가짜)） — 사유', 3, 'x').stamped, true);

  // 번호 뒤 경계: "ADR-0003X" 는 ADR-0003 도장이 아니다 → 진짜 도장이 붙어야 한다.
  const bogus = lib.stampPartialStatusLine('- 상태: 확정 · 부분 폐기 by ADR-0003X (가짜)', 3, 'x');
  assert.equal(bogus.stamped, true, 'ADR-0003X 를 ADR-0003 도장으로 오인해 진짜 도장을 누락');
  assert.ok(bogus.line.endsWith(' · 부분 폐기 by ADR-0003 (x)'), bogus.line);
});

test('18. D1 불균형 head(닫히지 않은 여는 괄호): 중복 도장 절대 없음', () => {
  const O = mkFixture('O1', {
    '0001-옛-결정.md': DASH_ADR(1, '옛 결정', '확정 (미닫힘 · 부분 폐기 by ADR-0002 (조항)'),
  });
  const f = path.join(O, '0001-옛-결정.md');
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 U', '--title', '개정', '--dir', O]);
  assert.equal(r.code, 0, r.stdout);
  // 불균형 head → 보수적 폴백(원문 전체 스캔): 이미 있는 ADR-0002 도장을 보고 스킵한다.
  assert.deepEqual(r.json.statusStamp, { stamped: false, position: null, reason: 'unbalanced-head-conservative' });
  assert.equal(countOf(statusLineOf(f), '부분 폐기 by ADR-0002'), 1, `중복 도장: ${statusLineOf(f)}`);
  assert.match(relatedLineOf(f), /Amended by ADR-0002 \(조항 U\)$/); // 관련줄 링크는 종전대로

  // 함수 레벨: 도장 없는 불균형 head 는 1회 찍히고, 같은 번호 재호출은 절대 두 번째를 안 만든다.
  const bare = '- 상태: 확정 (미닫힘';
  const once = lib.stampPartialStatusLine(bare, 2, '조항 U');
  assert.equal(once.stamped, true);
  assert.equal(countOf(once.line, '부분 폐기 by ADR-0002'), 1);
  const twice = lib.stampPartialStatusLine(once.line, 2, '조항 U');
  assert.equal(twice.stamped, false);
  assert.equal(twice.reason, 'unbalanced-head-conservative');
  assert.equal(countOf(twice.line, '부분 폐기 by ADR-0002'), 1, `재호출에서 중복: ${twice.line}`);
});

test('19. D2 공백 치환 합성 매치 없음: "· (도장 아님) 부분 폐기 by ADR-N" 은 도장이 아님', () => {
  const Q = mkFixture('Q1', {
    '0001-옛-결정.md': DASH_ADR(1, '옛 결정', '확정 · (도장 아님) 부분 폐기 by ADR-0002 (가짜)'),
  });
  const f = path.join(Q, '0001-옛-결정.md');
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 V', '--title', '개정', '--dir', Q]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(r.json.statusStamp.stamped, true, '가짜 문구를 도장으로 오인해 스킵함');
  assert.equal(
    statusLineOf(f),
    '- 상태: 확정 · (도장 아님) 부분 폐기 by ADR-0002 (가짜) · 부분 폐기 by ADR-0002 (조항 V)',
  );
  // 진짜 도장이 붙은 뒤에는 같은 번호 재도장을 스킵한다(깊이 0 매치).
  const after = lib.stampPartialStatusLine(statusLineOf(f), 2, '조항 V');
  assert.equal(after.stamped, false);
  assert.equal(after.reason, 'already-stamped');
  assert.equal(run(['lint', '--anchor-roots', '', '--dir', Q]).json.errorCount, 0);
});

test('20. D3 괄호 종류 엇갈림: 양방향 모두 진입부 거부 + 파일 무변경', () => {
  const R = mkFixture('R1', { '0001-옛-결정.md': DASH_ADR(1, '옛 결정') });
  const f = path.join(R, '0001-옛-결정.md');
  const before = read(f);
  for (const clause of ['조항 （mismatch)', '조항 (mismatch）']) {
    const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', clause, '--title', '개정', '--dir', R]);
    assert.equal(r.code, 1, `엇갈린 괄호가 통과함: "${clause}" → ${r.stdout}`);
    assert.match(r.json.error, /괄호가 안 맞음/);
    assert.match(r.json.error, /종류가 다르거나/);
    assert.equal(read(f), before, `거부인데 파일 변형: "${clause}"`);
    assert.equal(inDir(R, 2), null, `거부인데 새 파일 생성: "${clause}"`);
  }
  // 같은 종류로 맞춘 조항은 통과(반각·전각 각각).
  assert.equal(run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 (반각)', '--title', '개정 A', '--dir', R]).code, 0);
  assert.equal(run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 （전각）', '--title', '개정 B', '--dir', R]).code, 0);
});

test('14. F2 ADR_LIB_ONLY 매트릭스: 1/true 만 lib 모드(+stderr 알림), 나머지는 CLI 정상', () => {
  const J = mkFixture('J1', { '0001-결정.md': DASH_ADR(1, '결정') });
  const args = ['lint', '--anchor-roots', '', '--dir', J];
  for (const v of [undefined, '', '0', 'false', 'FALSE']) {
    const r = run(args, undefined, { ADR_LIB_ONLY: v });
    assert.equal(r.code, 0, `ADR_LIB_ONLY=${String(v)} 에서 exit!=0`);
    assert.ok(r.json && r.json.ok === true, `ADR_LIB_ONLY=${String(v)} 에서 CLI가 조용히 no-op: stdout="${r.stdout}"`);
  }
  for (const v of ['1', 'true', 'TRUE']) {
    const r = run(args, undefined, { ADR_LIB_ONLY: v });
    assert.equal(r.stdout.trim(), '', `lib 모드인데 CLI 출력 있음: ${r.stdout}`);
    assert.match(r.stderr, /ADR_LIB_ONLY: CLI entry skipped/, 'lib 모드 알림이 stderr 에 없음(조용한 누수)');
  }
});

test('15. F3 전각 괄호（）: 깊이 계산·조항 균형 검사 모두 인식', () => {
  const K = mkFixture('K1', {
    '0001-전각-결정.md': DASH_ADR(1, '전각 결정', '확정 （2026-07-01, 근거: 사용자 결정 — 리뷰） — 단, X 재검토'),
  });
  const f = path.join(K, '0001-전각-결정.md');
  const r = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 P', '--title', '개정', '--dir', K]);
  assert.equal(r.code, 0, r.stdout);
  assert.equal(
    statusLineOf(f),
    '- 상태: 확정 （2026-07-01, 근거: 사용자 결정 — 리뷰） · 부분 폐기 by ADR-0002 (조항 P) — 단, X 재검토',
    '전각 괄호 안 em-dash 를 단서절 경계로 오인(괄호 내용 훼손)',
  );
  // 전각 괄호 불균형 조항은 거부.
  const bad = run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 （미완', '--title', '개정', '--dir', K]);
  assert.equal(bad.code, 1, `전각 불균형 조항이 통과함: ${bad.stdout}`);
  assert.match(bad.json.error, /괄호가 안 맞음/);
});

test('16. F4 인덱스 조항 잘림: 파생 출력은 그대로 + 경고로 노출 + 재실행 멱등', () => {
  const L = mkFixture('L1', { '0001-옛-결정.md': DASH_ADR(1, '옛 결정') });
  assert.equal(run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '결정 2(배치) → 우클릭 메뉴', '--title', '개정', '--dir', L]).code, 0);

  const c = run(['index', '--check', '--anchor-roots', '', '--dir', L]);
  const w = c.json.warnings.filter((x) => x.kind === 'index-clause-truncated');
  assert.equal(w.length, 1, `잘림 경고 없음: ${JSON.stringify(c.json.warnings)}`);
  assert.equal(w[0].num, 1);
  assert.ok(w[0].clause.includes('('), '경고가 잘린 조항을 안 담음');

  // 괄호 없는 조항엔 경고가 안 뜬다(과잉 경고 방지).
  const M = mkFixture('L2', { '0001-옛-결정.md': DASH_ADR(1, '옛 결정') });
  assert.equal(run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '결정 2 배치', '--title', '개정', '--dir', M]).code, 0);
  assert.equal(run(['index', '--check', '--anchor-roots', '', '--dir', M]).json.warnings.filter((x) => x.kind === 'index-clause-truncated').length, 0);

  // G2: 전각 괄호만 든 조항은 캡처가 안 끊기므로 경고 대상이 아니다(거짓 양성 금지).
  const P = mkFixture('L3', { '0001-옛-결정.md': DASH_ADR(1, '옛 결정') });
  assert.equal(run(['supersede', '--old', '1', '--mode', 'partial', '--clause', '조항 （전각 괄호）', '--title', '개정', '--dir', P]).code, 0);
  const pc = run(['index', '--check', '--anchor-roots', '', '--dir', P]);
  assert.equal(pc.json.warnings.filter((x) => x.kind === 'index-clause-truncated').length, 0, '전각 전용 조항에 거짓 잘림 경고');
  run(['index', '--write', '--anchor-roots', '', '--dir', P]);
  const prow = read(path.join(P, 'README.md')).split(/\r?\n/).find((x) => x.startsWith('| [0001]'));
  assert.ok(prow.includes('조항 （전각 괄호）'), `전각 조항이 셀에서 잘림: ${prow}`); // 실제로 온전히 들어간다

  // 경고가 떠도 write 는 정상·멱등(출력 자체는 종전과 동일).
  assert.equal(run(['index', '--write', '--anchor-roots', '', '--dir', L]).json.changed, true);
  const after1 = read(path.join(L, 'README.md'));
  assert.equal(run(['index', '--write', '--anchor-roots', '', '--dir', L]).json.changed, false);
  assert.equal(read(path.join(L, 'README.md')), after1);
});

// ── 케이스 7: 실데이터 회귀(read-only, 옵션) ──────────────────────────────────
function hashDir(d) {
  const h = crypto.createHash('sha256');
  for (const n of fs.readdirSync(d).sort()) {
    const p = path.join(d, n);
    if (fs.statSync(p).isFile()) { h.update(n); h.update(fs.readFileSync(p)); }
  }
  return h.digest('hex');
}
const regRoot = process.env.ADR_REGRESSION_DIR;
const regDecisions = regRoot ? path.join(regRoot, 'docs', 'decisions') : null;
const regInfo = {};
if (!regRoot) {
  skip('7. 실데이터 회귀', 'ADR_REGRESSION_DIR 미설정(옵션 단계)');
} else if (!fs.existsSync(regDecisions)) {
  skip('7. 실데이터 회귀', `${path.join('<ADR_REGRESSION_DIR>', 'docs', 'decisions')} 없음`);
} else {
  test('7. 실데이터 회귀(read-only): lint 오류 0 + 베이스라인 동일 + 무변경', () => {
    const expect = process.env.ADR_REGRESSION_EXPECT ? JSON.parse(process.env.ADR_REGRESSION_EXPECT) : {};
    const before = hashDir(regDecisions);
    // 플래그 없이 프로젝트 루트에서 = 실운용 조건(기본값 docs/decisions + 코드 앵커 스캔).
    const l = run(['lint'], regRoot);
    assert.equal(l.code, 0, l.stdout);
    Object.assign(regInfo, { count: l.json.count, errorCount: l.json.errorCount, advisoryCount: l.json.advisoryCount });
    assert.equal(l.json.errorCount, 0, JSON.stringify(l.json.findings.filter((x) => !x.advisory)));
    if (expect.adrCount !== undefined) assert.equal(l.json.count, expect.adrCount);
    if (expect.lintAdvisories !== undefined) assert.equal(l.json.advisoryCount, expect.lintAdvisories, '권고 건수가 베이스라인과 다름');

    const i = run(['index', '--check'], regRoot);
    assert.equal(i.code, 0, i.stdout);
    const kinds = {};
    for (const w of i.json.warnings) kinds[w.kind] = (kinds[w.kind] || 0) + 1;
    Object.assign(regInfo, { indexClean: i.json.clean, indexDiffs: i.json.diffs.length, indexWarnings: i.json.warnings.length, warningKinds: kinds });
    // diffs = 파생 *출력*이 기존 인덱스와 다른 건수 → 이 값이 베이스라인과 같으면 셀 바이트가 안 바뀐 것.
    if (expect.indexDiffs !== undefined) assert.equal(i.json.diffs.length, expect.indexDiffs, 'index diff 건수가 베이스라인과 다름(파생 출력이 바뀜)');
    if (expect.indexWarnings !== undefined) assert.equal(i.json.warnings.length, expect.indexWarnings, 'index 경고 건수가 베이스라인과 다름');
    // 잘림 경고는 이번 라운드 신설 — 기대치를 주면 건수까지 고정한다(출력 불변 + 경고만 증가를 증명).
    if (expect.indexTruncationWarnings !== undefined) {
      assert.equal(kinds['index-clause-truncated'] ?? 0, expect.indexTruncationWarnings, '잘림 경고 건수가 기대와 다름');
    }

    assert.equal(hashDir(regDecisions), before, '회귀 대상 파일이 변경됨(read-only 위반)');
  });
}

// ── 요약 ──────────────────────────────────────────────────────────────────────
const failed = results.filter((r) => !r.ok);
const skipped = results.filter((r) => r.skipped);
console.log(`\n${results.length - failed.length - skipped.length}/${results.length - skipped.length} PASS` + (skipped.length ? ` (${skipped.length} SKIP)` : ''));
if (Object.keys(regInfo).length) console.log(`regression: ${JSON.stringify(regInfo)}`);
if (failed.length) { console.log(`픽스처 보존: ${TMP}`); process.exit(1); }
fs.rmSync(TMP, { recursive: true, force: true });

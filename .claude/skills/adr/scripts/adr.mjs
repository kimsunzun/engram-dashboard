// ADR 자동화 — 결정적 서기 작업(채번·템플릿·인덱스 재생성·supersede 양방향·lint).
// node 18+ 내장만 사용(외부 의존 0). 출력은 JSON(cdp.mjs 결.). 본문 prose·전체/부분 폐기 판단은
// 호출자(adr 스킬/LLM)가 한다 — 이 스크립트는 "기계가 틀리지 않게 할 수 있는 일"만 한다. (리서치: 기계/판단 경계)
//
// CLI:
//   node <경로>/adr.mjs new       --title "<제목>" [--status 확정|제안] [--dir <폴더>]
//   node <경로>/adr.mjs supersede --old <N> --mode full    --title "<제목>" [--status ...] [--dir ...]
//   node <경로>/adr.mjs supersede --old <N> --mode partial --clause "<바뀐 조항>" --title "<제목>" [--dir ...]
//   node <경로>/adr.mjs index  [--check | --write] [--dir <폴더>]
//   node <경로>/adr.mjs lint   [--dir <폴더>]
//
// 안전: index 기본 = --check(read-only diff만, README 안 고침). new/supersede는 쓰기.
//   --dir 로 대상 폴더를 바꿔 실데이터 격리(임시 폴더 dry-run). 기본 = <cwd>/docs/decisions/.
//   ADR_DIR 환경변수로도 지정 가능(--dir 우선).
//
// ── 멀티 소비처(프로젝트) 파라미터 ────────────────────────────────────────────
// 이 스크립트는 스킬 폴더에 상주하며 여러 소비처를 하나로 섬긴다. 모든 기본값은
//   dashboard(docs/decisions/, README.md 인덱스, 확정/제안/폐기/거부 어휘, 대시보드 템플릿)
//   동작을 그대로 보존한다 — 플래그 없이 dashboard 루트에서 돌리면 종전과 동일.
//   프로젝트별 실값은 스킬 바인딩(소비처 프로젝트 트리 .claude/skill-bindings/adr.md — ADR-0004)이 아래 플래그로 주입한다:
//   --dir <폴더> · --root <스캔/상대경로 기준(기본 cwd)> · --index-name <파일명(기본 README.md)>
//   --template <스캐폴드 템플릿 파일> · --status-vocab a,b,c · --default-status <값>
//   --anchor-roots a,b (코드 앵커 스캔 루트; 비우면 앵커 스캔 생략)
import fs from 'node:fs';
import path from 'node:path';

// ── 기본값 (전부 dashboard 동작 보존) ────────────────────────────────────────
const DEFAULT_STATUS_VOCAB = ['확정', '제안', '폐기', '거부'];
const DEFAULT_INDEX_NAME = 'README.md';
const DEFAULT_ANCHOR_ROOTS = ['crates', 'src', 'src-tauri', 'scripts'];
const DEFAULT_DEFAULT_STATUS = '확정';

// ── 인자 파싱 ────────────────────────────────────────────────────────────────
// 값 검증: `--flag` 다음 토큰이 없거나 `--`로 시작하면 값으로 흡수하지 않고 에러.
//   (안 막으면 `--title --status` 가 title="--status" 로 빨려 들어가 잘못된 제목·슬러그를 만든다.)
//   parseArgs는 fail(process.exit)을 직접 부르지 않고 errors를 모아 진입부에서 처리(테스트 가능성).
function parseArgs(argv) {
  const cmd = argv[0];
  const opts = {};
  const errors = [];
  for (let i = 1; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--check') opts.check = true;
    else if (a === '--write') opts.write = true;
    else if (a.startsWith('--')) {
      const key = a.slice(2);
      const next = argv[i + 1];
      // 값 토큰이 없거나(끝) `--`로 시작하면(다음 플래그) = 이 플래그의 값 누락.
      if (next === undefined || next.startsWith('--')) { errors.push(`${a} 값 누락`); continue; }
      opts[key] = next;
      i++;
    }
  }
  // --check 와 --write 는 상호배타(둘 다 켜면 의도 모호 → 거부).
  if (opts.check && opts.write) errors.push('--check 와 --write 는 동시 지정 불가(상호배타)');
  return { cmd, opts, errors };
}

// 제목/조항/상태 같은 단일행 필드에 파이프·개행이 오면 거부.
//   `|` 는 인덱스 표 칼럼을 쪼개 다음 index --write 에서 제목/상태가 영구 손실되고,
//   개행(\r/\n)은 H1·상태줄·관련줄을 쪼개 파싱을 깨거나 인덱스 제목을 자른다.
//   이스케이프보다 거부가 안전 — ADR 제목·조항에 파이프·개행은 비정상 입력이다.
function assertSingleLineField(label, value) {
  if (value === undefined || value === null) return;
  if (value.includes('|')) fail(`${label} 에 파이프('|') 문자 불가 — 인덱스 표 칼럼을 깨뜨림. 제거 후 재시도.`);
  if (/[\r\n]/.test(value)) fail(`${label} 에 개행(\\r/\\n) 불가 — H1/상태줄을 쪼갬. 한 줄로 재시도.`);
}

// 스캔/상대경로 기준 루트. 스크립트가 스킬 폴더로 이사했으므로 __dirname 기준을 못 쓴다 —
//   실행자가 워크스페이스 루트에서 돌린다는 바인딩 규약에 맞춰 cwd 기준(=dashboard 루트에서 종전과 동일).
function repoRoot(opts) {
  return path.resolve(opts.root || process.env.ADR_ROOT || process.cwd());
}

function adrDir(opts) {
  const d = opts.dir || process.env.ADR_DIR || path.join(repoRoot(opts), 'docs', 'decisions');
  return path.resolve(d);
}

// 프로젝트 파라미터를 한곳에서 해석(전부 dashboard 기본값으로 폴백).
function resolveConfig(opts) {
  const vocab = opts['status-vocab']
    ? opts['status-vocab'].split(',').map((s) => s.trim()).filter(Boolean)
    : DEFAULT_STATUS_VOCAB;
  const anchorRoots = opts['anchor-roots'] !== undefined
    ? opts['anchor-roots'].split(',').map((s) => s.trim()).filter(Boolean)
    : DEFAULT_ANCHOR_ROOTS;
  return {
    dir: adrDir(opts),
    root: repoRoot(opts),
    indexName: opts['index-name'] || DEFAULT_INDEX_NAME,
    statusVocab: vocab,
    defaultStatus: opts['default-status'] || DEFAULT_DEFAULT_STATUS,
    anchorRoots,
    templatePath: opts.template ? path.resolve(opts.template) : null,
  };
}

function fail(msg) { console.log(JSON.stringify({ ok: false, error: msg }, null, 2)); process.exit(1); }
function out(obj) { console.log(JSON.stringify({ ok: true, ...obj }, null, 2)); }

// ── ADR 파일 스캔/파싱 ───────────────────────────────────────────────────────
// 파일명 NNNN-*.md 만 ADR로 인식(README.md 등 제외).
const ADR_FILE_RE = /^(\d{4})-.*\.md$/;

function listAdrFiles(dir) {
  if (!fs.existsSync(dir)) fail(`ADR 폴더 없음: ${dir}`);
  return fs.readdirSync(dir)
    .map((name) => { const m = name.match(ADR_FILE_RE); return m ? { num: parseInt(m[1], 10), file: name } : null; })
    .filter(Boolean)
    .sort((a, b) => a.num - b.num);
}

// 상태줄에서 "상태 어휘"만 뽑는다. 단서절(em-dash 뒤, 또는 부분폐기 자유서술)은 무시.
// 데이터 변형: "확정 (...)", "**제안**", "**제안(Proposed)**", "폐기 (Superseded by ADR-N)",
//   "**폐기 (Superseded by ADR-N)** — ... ~~확정~~"(전체폐기), "확정 (...) — **단, ...폐기...**"(부분폐기).
// 규칙:
//   ① 첫 em-dash(—) 이전 부분(head)에서만 어휘를 찾는다 → 부분폐기 단서절의 "폐기"를 거짓검출 안 함.
//   ② 어휘는 head 의 *선두 토큰* 으로 앵커 매치한다(includes 금지). "미확정"·"확정안" 같은 비어휘가
//      "확정"으로 통과하면 안 된다. 한국어엔 \b 가 안 통하므로 어휘 뒤에 한글 음절이 안 오는지로 경계를 본다.
function extractStatusVocab(statusLineBody, vocab = DEFAULT_STATUS_VOCAB) {
  const head = statusLineBody.split('—')[0]; // em-dash 단서절 컷
  // 마크다운 강조·취소선 제거 후 앞 공백 제거 → 선두 어휘가 문자열 맨 앞에 오게.
  const stripped = head.replace(/\*\*/g, '').replace(/~~.*?~~/g, '').replace(/^\s+/, '');
  // 선두 어휘 앵커: 문자열 시작에서 어휘 매치 + 그 직후가 한글 음절이 아니어야 함(공백/괄호/구두점/끝).
  //   (어휘+한글 = "확정안" 같은 합성어 → 비어휘로 본다.) 어휘 목록은 프로젝트별 주입(기본 = dashboard).
  //   긴 어휘 우선(정렬)으로 접두 충돌 방지.
  const alt = [...vocab].sort((a, b) => b.length - a.length).map((v) => v.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|');
  const m = stripped.match(new RegExp(`^(${alt})(?![가-힣])`));
  return m ? m[1] : null; // 어휘 없음 = 형식 위반 후보
}

// 상태 줄에서 "상태:" 뒤 값 부분만 뽑는다. 두 형식 지원:
//   ① dashboard 단독 줄: "- 상태: 확정 (...) — ..."           → "확정 (...) — ..."
//   ② factory 결합 메타 줄: "- 날짜: ... · 상태: 채택 · 결정자: 사용자" → "채택"( · 구분자 앞까지)
//   결합 줄에선 다음 " · " 구분자 전까지만 값으로 본다(뒤 필드 흡수 방지).
function extractStatusSegment(line) {
  const single = line.match(/^-\s*상태:\s*(.*)$/); // 상태가 줄의 첫 필드(dashboard)
  if (single) return single[1].trim();
  const embedded = line.match(/·\s*상태:\s*([^·]*)/); // 결합 메타 줄 안의 상태 필드(factory)
  if (embedded) return embedded[1].trim();
  return null;
}

// 한 ADR 파일 헤더 파싱: H1 제목 · 상태줄 값 · 상태 어휘 · 관련줄 전문.
function parseAdr(dir, file, cfg = {}) {
  const vocab = cfg.statusVocab || DEFAULT_STATUS_VOCAB;
  const full = path.join(dir, file);
  const text = fs.readFileSync(full, 'utf8');
  const lines = text.split(/\r?\n/);
  const num = parseInt(file.match(ADR_FILE_RE)[1], 10);

  let title = null, statusLine = null, relatedLine = null, statusLineIdx = -1, relatedLineIdx = -1;
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    if (title === null) {
      const h = l.match(/^#\s+ADR-(\d+):\s*(.*)$/);
      if (h) { title = h[2].trim(); continue; }
    }
    // 상태 필드는 단독 줄(dashboard)이든 결합 메타 줄(factory)이든 "- ...상태:..." 불릿에서 추출.
    if (statusLine === null && /^-\s/.test(l) && /상태:/.test(l)) {
      const seg = extractStatusSegment(l);
      if (seg !== null) { statusLine = seg; statusLineIdx = i; }
    }
    if (relatedLine === null && /^-\s*관련:/.test(l)) { relatedLine = l.replace(/^-\s*관련:\s*/, '').trim(); relatedLineIdx = i; }
    if (title !== null && statusLine !== null && relatedLine !== null) break;
  }
  return {
    num, file, text, lines, title, statusLine, relatedLine, statusLineIdx, relatedLineIdx,
    vocab: statusLine ? extractStatusVocab(statusLine, vocab) : null,
  };
}

// 파일명 슬러그: 한국어는 유지, 영문 소문자화, 공백→-, 그 외 특수문자 제거. 결정적.
function slugify(title) {
  return title
    .trim()
    .toLowerCase()
    .replace(/[\s_]+/g, '-')                       // 공백·언더스코어 → 하이픈
    .replace(/[^0-9a-zㄱ-ㅎㅏ-ㅣ가-힣-]/g, '')  // 영숫자·한글(완성형+자모)·하이픈만 남김
    .replace(/-+/g, '-')                            // 연속 하이픈 축약
    .replace(/^-|-$/g, '');                         // 양끝 하이픈 제거
}

const pad4 = (n) => String(n).padStart(4, '0');

// ── 템플릿 ────────────────────────────────────────────────────────────────────
// 기본 = dashboard 내장 템플릿(정본 = docs/decisions/README.md "템플릿" 절과 섹션 구조 동일).
//   프로젝트가 --template <파일>로 다른 스캐폴드를 주입할 수 있다(예: factory 경량 템플릿).
//   템플릿 파일 플레이스홀더: {{NUM}} {{TITLE}} {{STATUS}} {{DATE}} {{RELATED}}.
//   본문 prose 슬롯은 비워 둔다 — 채우는 건 스킬/LLM(결정 날조 금지). 섹션 구조만 스캐폴드.
const DASHBOARD_TEMPLATE = `# ADR-{{NUM}}: {{TITLE}}

- 상태: {{STATUS}} ({{DATE}}, 근거: TODO)
- 관련: {{RELATED}}

## 맥락
TODO — 무슨 문제를 풀어야 했나.

## 결정
TODO — 무엇으로 정했나.

## 거부한 대안
- TODO 대안 A — 왜 버렸나.
- TODO 대안 B — 왜 버렸나.

## 근거
TODO — 실측·리뷰 등 결정의 뒷받침.

## 영향 / 불변식
TODO — 이 결정이 묶는 코드·게이트. 어기면 무엇이 깨지나.
`;

function scaffold({ num, title, status, related }, cfg = {}) {
  const today = new Date().toISOString().slice(0, 10);
  const tmpl = cfg.templatePath
    ? fs.readFileSync(cfg.templatePath, 'utf8')
    : DASHBOARD_TEMPLATE;
  return tmpl
    .replaceAll('{{NUM}}', pad4(num))
    .replaceAll('{{TITLE}}', title)
    .replaceAll('{{STATUS}}', status)
    .replaceAll('{{DATE}}', today)
    .replaceAll('{{RELATED}}', related || 'TODO — CLAUDE.md §X · <파일:라인> · step-log SN');
}

// 다음 번호: 폴더 파일 max+1. 쓰기 직전 재스캔으로 호출(race 완화).
function nextNum(dir) {
  const files = listAdrFiles(dir);
  return files.length ? files[files.length - 1].num + 1 : 1;
}

// ── 인덱스 재생성 ─────────────────────────────────────────────────────────────
// README의 "## 인덱스" 헤더 다음 표를 본문 헤더에서 재생성한다(본문=단일 출처 → 제목/상태 drift 차단).
// 단, 인덱스 기존 행의 *수작업 단서절*(본문 상태 어휘 뒤 괄호 설명, 부분폐기 등)은
// 본문에서 복원 불가하면 보존한다(자동 손실 금지). check 모드는 diff만, write만 실제 갱신.

const INDEX_HEADER = '## 인덱스';
// 인덱스 파일명은 프로젝트별(cfg.indexName, 기본 README.md).

// 본문에서 인덱스 한 행의 "상태 칸"을 만든다.
// 우선순위: ① 관련줄에 Amended by 링크(부분폐기) → "<어휘> (부분 폐기 by ADR-N: <clause>)"
//          ② 없으면 본문 상태줄에서 단서절을 정돈해 쓰되, 안전하게 어휘 + 전체폐기 링크만.
// 복잡한 자유서술 단서절은 본문서 손실 없이 못 옮기므로 기존 인덱스 행 보존 신호를 낸다.
function deriveIndexStatus(adr, oldRowStatus) {
  const vocab = adr.vocab || '?';
  // 전체 폐기: 상태줄에 "Superseded by ADR-N"이 (단서절 밖, em-dash 앞) 있고 + 상태 어휘가 폐기일 때만.
  //   어휘=폐기 조건이 없으면, 살아있는 ADR의 head 에 하이픈 단서로 "Superseded by"가 섞일 때
  //   (예: "확정 - 일부는 Superseded by ADR-N") 살아있는 결정을 폐기로 오검출한다.
  const head = adr.statusLine ? adr.statusLine.split('—')[0] : '';
  const fullSup = head.match(/Superseded by ADR-(\d+)/i);
  if (fullSup && vocab === '폐기') return { status: `폐기 (Superseded by ADR-${pad4(parseInt(fullSup[1], 10))})`, derivedFully: true };

  // 부분 폐기: 관련줄에 "Amended by ADR-N (clause)" 가 있으면 인덱스에 단서를 합성.
  const amendedBy = (adr.relatedLine || '').match(/Amended by ADR-(\d+)\s*\(([^)]*)\)/i);
  if (amendedBy) {
    return { status: `${vocab} (부분 폐기 by ADR-${pad4(parseInt(amendedBy[1], 10))}: ${amendedBy[2].trim()})`, derivedFully: true };
  }

  // 상태줄에 부분폐기 단서절(em-dash 뒤 "Superseded in part")이 있는데 관련줄 링크는 없음
  //   = 마이그레이션 전 레거시. 본문서 손실 없이 인덱스 단서를 못 만든다 → 기존 행 보존 권고.
  if (adr.statusLine && /Superseded in part/i.test(adr.statusLine)) {
    return { status: oldRowStatus ?? vocab, derivedFully: false, legacyPartial: true };
  }

  // 위 정형 케이스 어디에도 안 걸린 일반 상태. 본문서 파생 가능한 건 어휘뿐이다.
  // 단, 기존 인덱스 셀이 어휘를 *포함하면서 더 길면* = 수작업 큐레이션 단서(0024 "확정 (C3은 0025가 폐기 ...)")일 수 있다.
  //   본문엔 그 단서가 정형으로 없으므로 재생성하면 손실 → 기존 셀 보존 + 손실위험 경고. (안전 최우선)
  if (oldRowStatus && oldRowStatus !== vocab && stripStatus(oldRowStatus).includes(vocab)) {
    return { status: oldRowStatus, derivedFully: false, curatedStatus: true };
  }

  return { status: vocab, derivedFully: true };
}

// 인덱스 status 셀에서 마크다운·취소선 벗겨 어휘 비교용 평문.
function stripStatus(s) { return s.replace(/\*\*/g, '').replace(/~~.*?~~/g, ''); }

// README에서 기존 인덱스 행 맵 {num: {title, status, rawLine}} 파싱.
function parseIndexRows(readmeText) {
  const rows = new Map();
  const lines = readmeText.split(/\r?\n/);
  let inIndex = false;
  for (const l of lines) {
    if (l.trim() === INDEX_HEADER) { inIndex = true; continue; }
    if (inIndex && /^##\s/.test(l)) break; // 다음 섹션에서 종료
    // | [0001](file.md) | 제목 | 상태 |
    const m = l.match(/^\|\s*\[(\d+)\]\(([^)]+)\)\s*\|\s*(.*?)\s*\|\s*(.*?)\s*\|/);
    if (m) rows.set(parseInt(m[1], 10), { title: m[3].trim(), status: m[4].trim(), rawLine: l, file: m[2] });
  }
  return rows;
}

// 본문에서 인덱스 표 전체를 재생성하고, README 갱신 텍스트 + diff/경고를 돌려준다.
function buildIndex(cfg) {
  const dir = cfg.dir;
  const readmePath = path.join(dir, cfg.indexName);
  const readmeText = fs.existsSync(readmePath) ? fs.readFileSync(readmePath, 'utf8') : null;
  const oldRows = readmeText ? parseIndexRows(readmeText) : new Map();
  const adrs = listAdrFiles(dir).map((f) => parseAdr(dir, f.file, cfg));

  const newRows = [];
  const diffs = [];
  const warnings = [];

  for (const adr of adrs) {
    const old = oldRows.get(adr.num);
    const der = deriveIndexStatus(adr, old?.status);
    const bodyTitle = adr.title ?? '(제목 파싱 실패)';
    let status = der.status;
    let title = bodyTitle;

    // status 보존 경고: 본문서 단서를 못 옮기는 케이스(레거시 부분폐기 / 수작업 큐레이션 단서)는
    //   기존 인덱스 셀을 보존하고 경고만 낸다(자동 손실 금지 — 안전 최우선).
    if (der.legacyPartial && old && old.status !== der.status) {
      status = old.status;
      warnings.push({ num: adr.num, kind: 'status-legacy-partial-preserve', field: 'status',
        msg: `ADR-${pad4(adr.num)}: 본문에 Amends 링크 없는 레거시 부분폐기 — 기존 인덱스 status 보존(자동 재생성 안 함). supersede --mode partial로 본문 양방향 링크를 박으면 자동화 가능.`,
        preserved: old.status });
    } else if (der.curatedStatus && old) {
      status = old.status;
      warnings.push({ num: adr.num, kind: 'status-curated-preserve', field: 'status',
        msg: `ADR-${pad4(adr.num)}: 인덱스 status가 본문 어휘("${adr.vocab}")보다 단서가 많음(수작업 큐레이션 추정) — 기존 셀 보존. 본문 상태줄/관련줄에 정형 폐기·Amends 링크가 없어 자동 파생 불가.`,
        preserved: old.status });
    }

    // title 보존: 인덱스 제목이 본문 H1과 다르면 — 인덱스가 큐레이션 요약일 수 있어 자동으로 덮지 않는다.
    //   본문=단일 출처 원칙이지만, 인덱스 제목엔 본문 H1에 없는 단서(폐기 관계 등)가 손으로 들어가 있음(0019/0020/0030 등).
    //   안전 최우선: 무엇이 맞는지(본문 vs 인덱스)는 사람 판단 → 보존 + drift 경고. 손실 자동화 금지.
    if (old && old.title !== bodyTitle) {
      title = old.title; // 기존 인덱스 제목 보존
      warnings.push({ num: adr.num, kind: 'title-drift-preserve', field: 'title',
        msg: `ADR-${pad4(adr.num)}: 인덱스 제목 ≠ 본문 H1 — 기존 인덱스 제목 보존(자동 덮어쓰기 안 함). 어느 쪽이 정본인지는 사람 판단.`,
        indexTitle: old.title, bodyTitle });
    }

    // 링크는 *실제 본문 파일명*(adr.file)으로 만든다 — 인덱스의 stale old?.file 을 보존하면
    //   파일이 rename 된 뒤 인덱스 링크가 깨진다. 본문 파일이 단일 출처.
    const link = `[${pad4(adr.num)}](${adr.file})`;
    newRows.push(`| ${link} | ${title} | ${status} |`);

    // diff 수집(check 보고용 — 보존 여부와 무관하게 본문 vs 인덱스 차이를 그대로 보고)
    if (old) {
      if (old.title !== bodyTitle) diffs.push({ num: adr.num, field: 'title', index: old.title, body: bodyTitle, action: 'preserved' });
      if (old.status !== status) diffs.push({ num: adr.num, field: 'status', index: old.status, derived: status, action: 'rewritten' });
    } else {
      diffs.push({ num: adr.num, field: 'missing-row', body: bodyTitle, action: 'added' });
    }
  }
  // 인덱스에만 있고 파일 없는 행
  for (const [num] of oldRows) {
    if (!adrs.find((a) => a.num === num)) diffs.push({ num, field: 'orphan-index-row' });
  }

  // README 텍스트에 표 교체.
  //   inIndex(=옛 표 블록 스킵 모드): INDEX_HEADER 다음부터 기존 표 줄(^|)과 그 사이/뒤 빈줄을 전부 버린다.
  //   표 블록이 끝나는 첫 "내용 있는 비표 줄"(다음 ## 섹션 등)을 만나면 그 *앞에 빈 줄 1개를 보장*하고 통과.
  //   ★표 뒤 빈줄 보존: 빈줄을 보장하지 않으면 GFM 표가 바로 다음 줄을 같은 표로 빨아들인다(인덱스가 말단이
  //    아닐 때 깨짐). 빈줄을 일괄 버린 뒤 1개만 재삽입 → 인덱스 말단/중간 모두에서 idempotent.
  let newReadme = null;
  if (readmeText !== null) {
    const lines = readmeText.split(/\r?\n/);
    const result = [];
    let inIndex = false;
    for (let i = 0; i < lines.length; i++) {
      const l = lines[i];
      if (l.trim() === INDEX_HEADER) {
        result.push(l, '', '| # | 제목 | 상태 |', '|---|---|---|', ...newRows);
        inIndex = true;
        continue;
      }
      if (inIndex) {
        // 옛 표 줄·빈줄은 버린다. 내용 있는 비표 줄을 만나면 표 블록 종료.
        if (/^\s*\|/.test(l) || l.trim() === '') continue;
        // 다음 콘텐츠 줄 앞에 빈 줄 1개 보장(표와 분리) 후 통과.
        result.push('', l);
        inIndex = false;
        continue;
      }
      result.push(l);
    }
    newReadme = result.join('\n');
    // 원본의 trailing newline 보존(없으면 추가 안 함) — 불필요한 EOF diff·비-idempotent 방지.
    if (readmeText.endsWith('\n') && !newReadme.endsWith('\n')) newReadme += '\n';
  }

  return { readmePath, readmeText, newReadme, diffs, warnings, count: adrs.length };
}

// ── 명령 핸들러 ───────────────────────────────────────────────────────────────
function cmdNew(opts, cfg) {
  if (!opts.title) fail('--title 필요');
  assertSingleLineField('--title', opts.title);
  assertSingleLineField('--related', opts.related);
  const dir = cfg.dir;
  const status = opts.status || cfg.defaultStatus;
  if (!cfg.statusVocab.includes(status)) fail(`--status 는 ${cfg.statusVocab.join('/')} 중 하나`);
  const num = nextNum(dir); // 쓰기 직전 재스캔
  const slug = slugify(opts.title);
  // 슬러그가 비면 NNNN-.md 같은 파일명이 생긴다(파싱·재실행 깨짐) → 거부.
  if (!slug) fail(`--title 에서 파일명 슬러그를 못 만듦(영숫자·한글이 없음): "${opts.title}"`);
  const file = `${pad4(num)}-${slug}.md`;
  const full = path.join(dir, file);
  if (fs.existsSync(full)) fail(`이미 존재: ${file}`);
  fs.writeFileSync(full, scaffold({ num, title: opts.title, status, related: opts.related }, cfg), 'utf8');
  out({ op: 'new', num, file, path: full, status, note: '본문 prose 슬롯은 TODO — 스킬/LLM이 채운다. 인덱스는 index --write 로 재생성.' });
}

function cmdSupersede(opts, cfg) {
  if (!opts.title) fail('--title 필요');
  if (!opts.old) fail('--old <N> 필요');
  assertSingleLineField('--title', opts.title);
  assertSingleLineField('--clause', opts.clause);
  const mode = opts.mode;
  if (mode !== 'full' && mode !== 'partial') fail('--mode full|partial 필요');
  if (mode === 'partial' && !opts.clause) fail('partial 은 --clause "<바뀐 조항>" 필요');
  const dir = cfg.dir;
  const oldNum = parseInt(opts.old, 10);
  const oldEntry = listAdrFiles(dir).find((f) => f.num === oldNum);
  if (!oldEntry) fail(`옛 ADR 없음: ${oldNum}`);

  // ★원자성(ADR 데이터 무손상): 옛 ADR을 *먼저 완전 검증*한 뒤에야 새 파일을 쓴다.
  //   (새 파일을 먼저 쓰고 옛 파일을 나중에 검증하면, 중간 실패 시 새 파일만 남는 반쪽 상태가 된다.)
  const oldAdr = parseAdr(dir, oldEntry.file, cfg);
  const oldLines = oldAdr.lines.slice();
  if (mode === 'full') {
    // 옛 ADR에 상태줄이 있어야 폐기 표시를 박을 수 있다.
    if (oldAdr.statusLineIdx < 0) fail(`옛 ADR-${oldNum} 에 "- 상태:" 줄이 없어 폐기 표시를 못 박음(수동 처리 필요)`);
    // full 폐기는 상태를 *독립 "- 상태:" 줄*로 재기록한다 — 결합 메타 줄(예: "- 날짜: … · 상태: … · 결정자: …")
    //   형식에는 안전하게 못 박으므로(다른 필드 손실) 거부하고 수동 처리로 넘긴다.
    if (!/^-\s*상태:/.test(oldAdr.lines[oldAdr.statusLineIdx])) {
      fail(`옛 ADR-${oldNum} 상태가 독립 "- 상태:" 줄이 아님(결합 메타 줄) — full supersede 자동화 불가(수동 처리 필요): ${oldAdr.lines[oldAdr.statusLineIdx].trim()}`);
    }
    // ★멱등성: 이미 전체폐기(상태줄에 "폐기 (Superseded by ADR-")면 재래핑 금지(취소선 ~~ 무한 중첩 방지).
    if (/폐기\s*\(Superseded by ADR-/i.test(oldAdr.statusLine)) {
      fail(`옛 ADR-${oldNum} 은 이미 전체폐기됨 — 재래핑 거부(취소선 중첩 방지): ${oldAdr.statusLine}`);
    }
  } else {
    // 부분 폐기: 옛 ADR에 관련줄이 있어야 Amended by 양방향 링크를 박는다.
    if (oldAdr.relatedLineIdx < 0) fail(`옛 ADR-${oldNum} 에 "- 관련:" 줄이 없어 Amended by 링크를 못 박음(수동 처리 필요)`);
  }

  // 새 ADR 스캐폴드 메타 검증(파일은 옛 ADR 검증을 통과한 뒤에만 쓴다).
  const status = opts.status || cfg.defaultStatus;
  if (!cfg.statusVocab.includes(status)) fail(`--status 는 ${cfg.statusVocab.join('/')} 중 하나`);
  const newNum = nextNum(dir);

  // partial 멱등성: 옛 관련줄에 동일 "Amended by ADR-N" 이 이미 있으면 중복 append 금지.
  //   (재실행 시 같은 링크를 두 번 박아 관련줄이 늘어나는 손상을 막는다. 번호 단위로 검출 — 같은 N이면 거부.)
  if (mode === 'partial') {
    const reExisting = new RegExp(`Amended by ADR-${pad4(newNum)}(?!\\d)`, 'i');
    if (reExisting.test(oldAdr.relatedLine)) {
      fail(`옛 ADR-${oldNum} 관련줄에 이미 "Amended by ADR-${pad4(newNum)}" 존재 — 중복 append 거부: ${oldAdr.relatedLine}`);
    }
  }

  const slug = slugify(opts.title);
  if (!slug) fail(`--title 에서 파일명 슬러그를 못 만듦(영숫자·한글이 없음): "${opts.title}"`);
  const newFile = `${pad4(newNum)}-${slug}.md`;
  const newFull = path.join(dir, newFile);
  if (fs.existsSync(newFull)) fail(`이미 존재: ${newFile}`);

  // 여기까지 도달 = 옛 ADR·새 메타 전부 검증 통과. 이제 새 파일을 쓴다(반쪽 상태 불가).
  const relLink = mode === 'full'
    ? `Supersedes ADR-${pad4(oldNum)}`
    : `Amends ADR-${pad4(oldNum)} (${opts.clause})`;
  fs.writeFileSync(newFull, scaffold({ num: newNum, title: opts.title, status, related: relLink + ' · TODO 나머지 관련' }, cfg), 'utf8');

  // 옛 ADR 변형(검증 시 확인한 줄·인덱스만 사용).
  if (mode === 'full') {
    // 기존 status 텍스트는 ~~취소선~~ 으로 이력 보존(0023 관습).
    const prevBody = oldAdr.statusLine; // "확정 (...)" 등
    oldLines[oldAdr.statusLineIdx] = `- 상태: **폐기 (Superseded by ADR-${pad4(newNum)})** — TODO 사유. ~~${prevBody}~~`;
  } else {
    const amend = `Amended by ADR-${pad4(newNum)} (${opts.clause})`;
    oldLines[oldAdr.relatedLineIdx] = `${oldLines[oldAdr.relatedLineIdx]} · ${amend}`;
  }
  fs.writeFileSync(path.join(dir, oldEntry.file), oldLines.join('\n'), 'utf8');

  out({
    op: 'supersede', mode, newNum, newFile, oldNum, oldFile: oldEntry.file,
    bidirectional: mode === 'full'
      ? { new: `Supersedes ADR-${pad4(oldNum)}`, old: `상태→폐기 (Superseded by ADR-${pad4(newNum)}), 기존 상태 취소선 보존` }
      : { new: `Amends ADR-${pad4(oldNum)} (${opts.clause})`, old: `상태 유지 + 관련줄에 Amended by ADR-${pad4(newNum)} (${opts.clause})` },
    note: '새 ADR 본문 prose + 옛 ADR의 TODO 사유는 스킬/LLM이 채운다. 인덱스는 index --write 로 재생성.',
  });
}

function cmdIndex(opts, cfg) {
  const { readmePath, readmeText, newReadme, diffs, warnings, count } = buildIndex(cfg);
  if (readmeText === null) fail(`인덱스 파일 없음: ${readmePath}`);

  const write = opts.write && !opts.check;
  if (write) {
    if (warnings.length) {
      // 손실 위험(레거시 부분폐기 보존)은 보존 로직으로 이미 처리됨 — 경고만 동반 출력.
    }
    if (newReadme !== readmeText) fs.writeFileSync(readmePath, newReadme, 'utf8');
    out({ op: 'index', mode: 'write', changed: newReadme !== readmeText, count, diffs, warnings });
  } else {
    // 기본 = check: 안 고치고 diff·경고만.
    out({ op: 'index', mode: 'check', clean: diffs.length === 0, count, diffs, warnings,
      hint: diffs.length ? 'index --write 로 본문 기준 재생성(legacy-partial-preserve 경고는 기존 단서 보존됨).' : '인덱스 정합.' });
  }
}

function cmdLint(opts, cfg) {
  const dir = cfg.dir;
  const adrs = listAdrFiles(dir).map((f) => parseAdr(dir, f.file, cfg));
  const findings = [];

  // ① 중복·빠진 번호
  const seen = new Map();
  for (const a of adrs) {
    if (seen.has(a.num)) findings.push({ kind: 'duplicate-number', num: a.num, files: [seen.get(a.num), a.file] });
    else seen.set(a.num, a.file);
  }
  if (adrs.length) {
    const max = adrs[adrs.length - 1].num;
    for (let n = 1; n <= max; n++) if (!seen.has(n)) findings.push({ kind: 'missing-number', num: n });
  }

  // ③ 상태 어휘 유효성 (어휘만 — 단서절 무시)
  for (const a of adrs) {
    if (!a.statusLine) findings.push({ kind: 'no-status-line', num: a.num, file: a.file });
    else if (!a.vocab) findings.push({ kind: 'invalid-status-vocab', num: a.num, file: a.file, statusLine: a.statusLine });
    if (a.title === null) findings.push({ kind: 'no-h1-title', num: a.num, file: a.file });
  }

  // ② supersede 양방향 일치 — 전체폐기: 옛 "폐기(Superseded by N)" ⟺ 새 "Supersedes M".
  //    관련줄/상태줄에서 양쪽 링크를 모으고 짝이 안 맞으면 보고.
  const supBy = new Map();   // oldNum -> newNum  (옛 상태줄 "Superseded by ADR-new")
  const supersedes = new Map(); // newNum -> oldNum (새 관련줄 "Supersedes ADR-old")
  const amendedBy = new Map(); // oldNum -> [newNum] (옛 관련줄 "Amended by ADR-new")
  const amends = new Map();   // newNum -> [oldNum] (새 관련줄 "Amends ADR-old")
  for (const a of adrs) {
    const sl = a.statusLine || '';
    const rl = a.relatedLine || '';
    let m;
    // 전체폐기 링크는 상태 어휘가 폐기이고 + 단서절 밖(em-dash 앞 head)에 있을 때만 인정.
    //   (어휘=폐기 + head 한정이 없으면 부분폐기 단서절 "Superseded in part" 나 하이픈 단서줄의
    //    살아있는 ADR을 전체폐기로 오검출해 거짓 양방향-불일치 에러를 낸다.)
    const slHead = sl.split('—')[0];
    if (a.vocab === '폐기' && (m = slHead.match(/Superseded by ADR-(\d+)/i))) supBy.set(a.num, parseInt(m[1], 10));
    const reSup = /Supersedes ADR-(\d+)/gi;
    while ((m = reSup.exec(rl))) { const k = a.num; const v = parseInt(m[1], 10); supersedes.set(k, [...(supersedes.get(k) || []), v]); }
    const reAB = /Amended by ADR-(\d+)/gi;
    while ((m = reAB.exec(rl))) { const k = a.num; const v = parseInt(m[1], 10); amendedBy.set(k, [...(amendedBy.get(k) || []), v]); }
    const reAm = /Amends ADR-(\d+)/gi;
    while ((m = reAm.exec(rl))) { const k = a.num; const v = parseInt(m[1], 10); amends.set(k, [...(amends.get(k) || []), v]); }
  }
  // full: 옛 Superseded by N ⟺ 새 N Supersedes 옛
  for (const [oldNum, newNum] of supBy) {
    const list = supersedes.get(newNum) || [];
    if (!list.includes(oldNum)) findings.push({ kind: 'supersede-unidirectional', detail: `ADR-${pad4(oldNum)}는 ADR-${pad4(newNum)}가 폐기했다고 하나, ADR-${pad4(newNum)} 관련줄에 "Supersedes ADR-${pad4(oldNum)}" 없음`, oldNum, newNum });
  }
  for (const [newNum, olds] of supersedes) for (const oldNum of olds) {
    if (supBy.get(oldNum) !== newNum) findings.push({ kind: 'supersede-unidirectional', detail: `ADR-${pad4(newNum)}가 ADR-${pad4(oldNum)} 폐기 주장하나, ADR-${pad4(oldNum)} 상태줄에 "Superseded by ADR-${pad4(newNum)}" 없음`, oldNum, newNum });
  }
  // partial: 옛 Amended by N ⟺ 새 Amends 옛 (양방향)
  for (const [oldNum, news] of amendedBy) for (const newNum of news) {
    if (!(amends.get(newNum) || []).includes(oldNum)) findings.push({ kind: 'amend-unidirectional', detail: `ADR-${pad4(oldNum)}는 ADR-${pad4(newNum)}가 일부 개정한다 하나, ADR-${pad4(newNum)} 관련줄에 "Amends ADR-${pad4(oldNum)}" 없음`, oldNum, newNum });
  }
  for (const [newNum, olds] of amends) for (const oldNum of olds) {
    if (!(amendedBy.get(oldNum) || []).includes(newNum)) findings.push({ kind: 'amend-unidirectional', detail: `ADR-${pad4(newNum)}가 ADR-${pad4(oldNum)} 일부 개정 주장하나, ADR-${pad4(oldNum)} 관련줄에 "Amended by ADR-${pad4(newNum)}" 없음`, oldNum, newNum });
  }

  // ②-b 레거시 부분폐기: 상태줄에 "Superseded in part" 자유서술은 있으나 양방향 Amends 링크가 없음 → 권고(거짓오류 아님).
  for (const a of adrs) {
    if (a.statusLine && /Superseded in part/i.test(a.statusLine)) {
      const hasLink = (amendedBy.get(a.num) || []).length > 0;
      if (!hasLink) findings.push({ kind: 'legacy-partial-no-link', num: a.num, file: a.file, advisory: true,
        detail: `ADR-${pad4(a.num)} 상태줄에 부분폐기 자유서술이 있으나 "Amended by ADR-N" 양방향 링크 없음(레거시). 마이그레이션 권고 — lint 오류 아님.` });
    }
  }

  // ④ 코드 앵커 고아 — // ADR-NNNN 을 코드 경로(crates/ src/)에서만 긁어 docs/ 제외.
  //    존재하지 않거나 폐기된 ADR을 코드가 아직 가리키면 후보.
  const validNums = new Set(adrs.map((a) => a.num));
  const deprecatedNums = new Set(adrs.filter((a) => a.vocab === '폐기').map((a) => a.num));
  const anchors = scanCodeAnchors(cfg);
  for (const an of anchors) {
    if (!validNums.has(an.num)) findings.push({ kind: 'orphan-anchor-missing', num: an.num, ref: an.ref, detail: `코드 앵커 // ADR-${pad4(an.num)} 가 존재하지 않는 ADR을 가리킴` });
    else if (deprecatedNums.has(an.num)) findings.push({ kind: 'orphan-anchor-deprecated', num: an.num, ref: an.ref, advisory: true, detail: `코드 앵커 // ADR-${pad4(an.num)} 가 폐기된 ADR을 가리킴(폐기된 결정을 코드가 아직 따를 수 있음 — 확인 권고)` });
  }

  const errors = findings.filter((f) => !f.advisory);
  out({ op: 'lint', clean: errors.length === 0, count: adrs.length, errorCount: errors.length, advisoryCount: findings.length - errors.length, findings });
}

// 코드 경로(crates/, src/)에서만 `// ADR-NNNN` 긁기. docs/ 제외(rg 노이즈 차단).
// node 내장만 — 디렉터리 재귀 직접 구현. 텍스트 파일만 정규식 스캔.
//   스캔 루트·기준 경로는 cfg(프로젝트별) — anchorRoots 가 비면 앵커 스캔 자체를 생략(코드 없는 소비처).
function scanCodeAnchors(cfg) {
  const base = cfg.root;
  if (!cfg.anchorRoots.length) return [];
  const roots = cfg.anchorRoots.map((d) => path.join(base, d)).filter((d) => fs.existsSync(d));
  const SKIP_DIR = new Set(['node_modules', 'target', 'dist', '.git', 'docs']);
  const CODE_EXT = new Set(['.rs', '.ts', '.tsx', '.js', '.jsx', '.mjs', '.css', '.toml', '.json']);
  const ANCHOR_RE = /\/\/\s*ADR-(\d+)/g; // 코드 주석 // ADR-NNNN 만(문서 본문 "ADR-" 참조 제외)
  const anchors = [];
  const walk = (dir) => {
    let ents;
    try { ents = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const e of ents) {
      if (SKIP_DIR.has(e.name)) continue;
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (CODE_EXT.has(path.extname(e.name))) {
        let text;
        try { text = fs.readFileSync(p, 'utf8'); } catch { continue; }
        let m;
        ANCHOR_RE.lastIndex = 0;
        while ((m = ANCHOR_RE.exec(text))) anchors.push({ num: parseInt(m[1], 10), ref: path.relative(base, p).replace(/\\/g, '/') });
      }
    }
  };
  for (const r of roots) walk(r);
  return anchors;
}

// ── 진입 ──────────────────────────────────────────────────────────────────────
const { cmd, opts, errors } = parseArgs(process.argv.slice(2));
if (errors.length) fail(`인자 오류: ${errors.join('; ')}`);
const cfg = resolveConfig(opts);
switch (cmd) {
  case 'new': cmdNew(opts, cfg); break;
  case 'supersede': cmdSupersede(opts, cfg); break;
  case 'index': cmdIndex(opts, cfg); break;
  case 'lint': cmdLint(opts, cfg); break;
  default:
    console.log(JSON.stringify({ ok: false, error: 'usage', usage: [
      'node <경로>/adr.mjs new --title "<제목>" [--status ...] [--dir <폴더>] [--template <파일>] [--index-name <이름>] [--status-vocab a,b,c] [--default-status <값>] [--root <루트>] [--anchor-roots a,b]',
      'node <경로>/adr.mjs supersede --old <N> --mode full --title "<제목>" [--dir <폴더>]',
      'node <경로>/adr.mjs supersede --old <N> --mode partial --clause "<조항>" --title "<제목>" [--dir <폴더>]',
      'node <경로>/adr.mjs index [--check | --write] [--dir <폴더>] [--index-name <이름>]',
      'node <경로>/adr.mjs lint [--dir <폴더>] [--status-vocab a,b,c] [--anchor-roots a,b]',
    ] }, null, 2));
    process.exit(1);
}

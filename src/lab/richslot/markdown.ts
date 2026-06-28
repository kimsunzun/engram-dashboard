// 미니 Markdown 파서 — 순수 TS(React 무관). 텍스트 → MdNode[] (블록 트리).
//
// ★왜 자체 구현★: 의존성 추가 금지(package.json 공유) + 사용자 #1 페인포인트("raw
// markdown 이 안 읽힌다") 해결이 목적. 풀 CommonMark 가 아니라 claude 출력에서 실제로
// 나오는 부분집합만 — 헤더/볼드/이탤릭/인라인코드/펜스코드/리스트/문단. 모르는 문법은
// 평문으로 흘려보낸다(robust fallback — 깨지느니 raw 로 보이는 게 낫다).
//
// ★층 분리★: 이 파일은 파싱만(MdNode 트리 산출). React 렌더는 markdown.tsx 가 담당.
// 그래서 파싱 단독으로 vitest 단언 가능(markdown.test.ts).

/** 인라인 조각 — 한 줄/문단 안의 서식. */
export type MdInline =
  | { type: 'text'; text: string }
  | { type: 'bold'; text: string }
  | { type: 'italic'; text: string }
  | { type: 'code'; text: string } // 인라인 `code`

/** diff 펜스 안의 한 줄 분류 — add(+)/del(-)/평범(ctx). 렌더가 CSS class 로 색칠. */
export type MdDiffKind = 'add' | 'del' | 'ctx'

/** 블록 노드 — 문단/헤더/리스트/펜스코드. */
export type MdNode =
  | { type: 'heading'; level: 1 | 2 | 3; inlines: MdInline[] }
  | { type: 'paragraph'; inlines: MdInline[] }
  | { type: 'list'; ordered: boolean; items: MdInline[][] }
  // 펜스 ```code``` — lang 은 info string. diff 면 줄마다 kind 태그(렌더가 색칠).
  | { type: 'code'; lang: string; code: string; isDiff: boolean; lines: { kind: MdDiffKind; text: string }[] }

const FENCE_RE = /^(```|~~~)(.*)$/

/**
 * 텍스트 → MdNode[] 블록 트리.
 * 라인 단위 상태기: 펜스 진입/탈출을 먼저 가르고, 펜스 밖에서만 헤더/리스트/문단을 본다.
 */
export function parseMarkdown(text: string): MdNode[] {
  const lines = text.split('\n')
  const nodes: MdNode[] = []
  let i = 0

  // 문단/리스트 누적 버퍼 — 빈 줄·블록 경계에서 flush.
  let para: string[] = []
  let listItems: string[] = []
  let listOrdered = false

  const flushPara = () => {
    if (para.length === 0) return
    nodes.push({ type: 'paragraph', inlines: parseInline(para.join('\n')) })
    para = []
  }
  const flushList = () => {
    if (listItems.length === 0) return
    nodes.push({ type: 'list', ordered: listOrdered, items: listItems.map(parseInline) })
    listItems = []
  }
  const flushAll = () => {
    flushPara()
    flushList()
  }

  while (i < lines.length) {
    const line = lines[i]
    const fence = FENCE_RE.exec(line.trimStart())

    // ── 펜스 코드 블록 ── (헤더/리스트보다 먼저 — 펜스 안에선 마크다운 해석 안 함)
    if (fence) {
      flushAll()
      const lang = fence[2].trim()
      const body: string[] = []
      i++ // 여는 펜스 소비
      while (i < lines.length && !FENCE_RE.test(lines[i].trimStart())) {
        body.push(lines[i])
        i++
      }
      i++ // 닫는 펜스 소비(없으면 EOF — 안전하게 종료)
      // diff 판정: lang==diff 이거나 본문에 +/- 시작 줄이 있으면 diff 로 취급해 줄 태깅.
      const looksDiff = lang.toLowerCase() === 'diff' || body.some((l) => /^[+-]/.test(l))
      const taggedLines = body.map((l) => ({ kind: diffKind(l, looksDiff), text: l }))
      nodes.push({ type: 'code', lang, code: body.join('\n'), isDiff: looksDiff, lines: taggedLines })
      continue
    }

    // ── 빈 줄 = 블록 경계 ──
    if (line.trim() === '') {
      flushAll()
      i++
      continue
    }

    // ── 헤더 (#/##/###) ──
    const h = /^(#{1,3})\s+(.*)$/.exec(line)
    if (h) {
      flushAll()
      nodes.push({ type: 'heading', level: h[1].length as 1 | 2 | 3, inlines: parseInline(h[2]) })
      i++
      continue
    }

    // ── 리스트 (- bullet / 1. numbered) ──
    const bullet = /^\s*[-*]\s+(.*)$/.exec(line)
    const numbered = /^\s*\d+\.\s+(.*)$/.exec(line)
    if (bullet || numbered) {
      flushPara() // 문단 진행 중이었으면 끊는다
      const ordered = !!numbered
      // 리스트 종류가 바뀌면(불릿↔번호) 기존 리스트 flush 후 새로 시작.
      if (listItems.length > 0 && ordered !== listOrdered) flushList()
      listOrdered = ordered
      listItems.push((bullet ?? numbered)![1])
      i++
      continue
    }

    // ── 그 외 = 문단 줄 누적 ──
    flushList() // 리스트 뒤 비-리스트 줄이면 리스트 닫고 문단 시작
    para.push(line)
    i++
  }

  flushAll()
  return nodes
}

/** diff 줄 분류 — diff 컨텍스트에서만 +/- 를 add/del 로(아니면 전부 ctx). */
function diffKind(line: string, isDiff: boolean): MdDiffKind {
  if (!isDiff) return 'ctx'
  // diff 헤더(+++/---)는 add/del 색칠 대상 아님 — 평범 처리.
  if (line.startsWith('+++') || line.startsWith('---')) return 'ctx'
  if (line.startsWith('+')) return 'add'
  if (line.startsWith('-')) return 'del'
  return 'ctx'
}

// 인라인 토큰: `code`(최우선 — 안쪽 **/* 무시), **bold**, *italic*.
// 순서가 중요 — code 를 먼저 떼서 코드 안의 별표가 볼드로 안 잡히게 한다.
const INLINE_RE = /(`[^`]+`)|(\*\*[^*]+\*\*)|(\*[^*]+\*)/

/** 한 덩어리 텍스트 → MdInline[] (서식 토큰 분해). 매칭 없으면 통째로 text 1개. */
export function parseInline(text: string): MdInline[] {
  const out: MdInline[] = []
  let rest = text
  let m: RegExpExecArray | null
  while ((m = INLINE_RE.exec(rest))) {
    if (m.index > 0) out.push({ type: 'text', text: rest.slice(0, m.index) })
    const tok = m[0]
    if (tok.startsWith('`')) out.push({ type: 'code', text: tok.slice(1, -1) })
    else if (tok.startsWith('**')) out.push({ type: 'bold', text: tok.slice(2, -2) })
    else out.push({ type: 'italic', text: tok.slice(1, -1) })
    rest = rest.slice(m.index + tok.length)
  }
  if (rest) out.push({ type: 'text', text: rest })
  // 전부 평문이면 빈 배열 회피(호출부가 항상 1개 이상 기대) — 빈 입력은 빈 text.
  return out.length > 0 ? out : [{ type: 'text', text }]
}

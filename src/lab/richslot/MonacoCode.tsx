// Monaco 기반 코드/diff 렌더 래퍼 — "VS Code 스타일" A/B 비교용(스파이크).
//
// ★왜 opt-in(lazy)★: Monaco 는 인스턴스마다 무겁다(에디터 1개 = 워커+토크나이저+레이아웃).
// 한 화면에 여러 개 깔면 jank 가 난다 — 그게 이 스파이크가 비교하려는 바로 그 비용이다.
// 그래서 자체 렌더(plain/inline)를 기본으로 두고, 사용자가 토글을 'monaco' 로 바꿀 때만
// 이 모듈을 dynamic import 한다(main.tsx 에서 React.lazy). 토글이 plain/inline 이면
// monaco-editor 청크 자체가 로드되지 않는다.
//
// ★loader 설정★: 앱 본체(src/components/diff/DiffPanel.tsx)와 동일하게 로컬 번들
// monaco-editor 를 쓴다 — `loader.config({ monaco })`. CDN 의존 없음(오프라인 동작).
//
// ★층 분리★: 파서(markdown.ts)는 Monaco 를 모른다(no-dep 순수 유지). Monaco 는 오직
// 이 React 렌더층, 그것도 토글 뒤에만 산다.

import { useRef, useState } from 'react'
import Editor, { DiffEditor, loader } from '@monaco-editor/react'
import * as monaco from 'monaco-editor'

// 앱 본체와 같은 로컬 번들 monaco 사용(네트워크 X). 모듈 1회 평가 시 등록.
loader.config({ monaco })

// 라인 높이(px) — auto-size 계산에 쓴다. Monaco 기본 19px 근처.
const LINE_HEIGHT = 19
// 인라인 흐름에 맞춘 상·하 여백 + 가로 스크롤바 여유.
const PADDING = 12
// 너무 큰 블록은 캡 — 인라인 채팅에서 한 에디터가 화면을 다 먹지 않게.
const MAX_HEIGHT = 600

// 읽기전용·미니맵off·최소크롬 공통 옵션. 인라인 흐름용(스크롤바 자동, 줄바꿈 off).
const BASE_OPTIONS: monaco.editor.IStandaloneEditorConstructionOptions = {
  readOnly: true,
  domReadOnly: true,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  lineNumbers: 'off',
  glyphMargin: false,
  folding: false,
  // 인라인이라 컨테이너 스크롤에 맡기고, 가로만 필요시 노출.
  scrollbar: { vertical: 'hidden', horizontal: 'auto', alwaysConsumeMouseWheel: false },
  overviewRulerLanes: 0,
  renderLineHighlight: 'none',
  contextmenu: false,
  fontSize: 12.5,
  padding: { top: 6, bottom: 6 },
}

function contentHeight(text: string): number {
  const lines = text.split('\n').length
  return Math.min(MAX_HEIGHT, lines * LINE_HEIGHT + PADDING)
}

/**
 * 펜스 코드 블록 1개를 read-only Monaco 로 렌더 → 실제 VS Code 하이라이팅.
 * lang 은 펜스 info-string. 비면 'plaintext'(Monaco 가 토큰 안 함).
 * 높이는 내용 줄수로 auto-size 해 인라인 채팅 흐름에 박힌다.
 */
export function MonacoCodeBlock({ code, lang }: { code: string; lang: string }) {
  const [height, setHeight] = useState(() => contentHeight(code))
  const language = normalizeLang(lang)
  return (
    <div className="md-code-monaco" style={{ height }}>
      <Editor
        value={code}
        language={language}
        theme="vs-dark"
        height={height}
        options={BASE_OPTIONS}
        // 마운트 후 실제 contentHeight 로 재조정(폰트/줄바꿈 변동 흡수).
        onMount={(editor) => {
          const h = Math.min(MAX_HEIGHT, editor.getContentHeight() + PADDING)
          if (h > 0) setHeight(h)
        }}
      />
    </div>
  )
}

/**
 * unified diff 텍스트를 Monaco 로 렌더 → VS Code 의 초록/빨강 diff.
 * ★왜 DiffEditor(인라인)★: 일반 Editor + language="diff" 는 vs-dark 의 diff 토큰색이
 * 거의 없어 무채색으로 보인다(사용자 지적). 진짜 +/- 배경색은 DiffEditor 가 그린다.
 * unified diff 를 original/modified 로 역복원해 넣고, renderSideBySide:false(인라인)로
 * 좁은 카드에서도 한 열에 삭제(빨강)·추가(초록) 줄 배경이 칠해지게 한다.
 */
export function MonacoDiff({ diff }: { diff: string }) {
  const { original, modified } = useRef(reconstructFromUnified(diff)).current
  // 인라인 diff 표시 줄 수 ≈ unified 본문(헤더 제외) 줄 수. 헤더는 화면에 안 나오므로 높이서 뺀다.
  const bodyLines = diff
    .split('\n')
    .filter((l) => !/^(diff |index |--- |\+\+\+ |@@)/.test(l)).length
  const [height, setHeight] = useState(() =>
    Math.min(MAX_HEIGHT, Math.max(bodyLines, 1) * LINE_HEIGHT + PADDING),
  )
  return (
    <div className="md-code-monaco md-diff-monaco" style={{ height }}>
      <DiffEditor
        original={original}
        modified={modified}
        theme="vs-dark"
        height={height}
        options={{ ...BASE_OPTIONS, renderSideBySide: false, lineNumbers: 'off' }}
        onMount={(editor) => {
          // 인라인 모드 높이 보정 — modified 에디터 contentHeight 기준(best-effort).
          const h = Math.min(MAX_HEIGHT, editor.getModifiedEditor().getContentHeight() + PADDING)
          if (h > 0) setHeight(h)
        }}
      />
    </div>
  )
}

/**
 * 진짜 side-by-side DiffEditor — unified diff 헝크에서 original/modified 를 역복원.
 * 현재 기본 경로(MonacoDiff)는 unified 라 이건 옵션. 헝크 파싱이 단순한 경우만 쓴다.
 */
export function MonacoDiffSideBySide({ diff }: { diff: string }) {
  const { original, modified } = useRef(reconstructFromUnified(diff)).current
  const lineCount = Math.max(original.split('\n').length, modified.split('\n').length)
  const height = Math.min(MAX_HEIGHT, lineCount * LINE_HEIGHT + PADDING)
  return (
    <div className="md-code-monaco md-diff-monaco" style={{ height }}>
      <DiffEditor
        original={original}
        modified={modified}
        theme="vs-dark"
        height={height}
        options={{ ...BASE_OPTIONS, renderSideBySide: true, lineNumbers: 'on' }}
      />
    </div>
  )
}

/** 펜스 info-string → Monaco language id. 빈/미지원은 plaintext 로 안전 강등. */
function normalizeLang(lang: string): string {
  const l = lang.trim().toLowerCase()
  if (!l) return 'plaintext'
  // 흔한 별칭 → Monaco id. 미등록 언어는 Monaco 가 plaintext 로 폴백하므로 그대로 넘겨도 안전.
  const alias: Record<string, string> = {
    py: 'python',
    sh: 'shell',
    bash: 'shell',
    zsh: 'shell',
    ts: 'typescript',
    tsx: 'typescript',
    js: 'javascript',
    jsx: 'javascript',
    rs: 'rust',
    md: 'markdown',
    yml: 'yaml',
  }
  return alias[l] ?? l
}

/**
 * unified diff → {original, modified} 역복원(side-by-side 용).
 * 헝크의 ' '(공통)·'-'(삭제)·'+'(추가) prefix 만 본다. diff/index/@@ 헤더는 스킵.
 * 완벽한 patch 적용이 아니라 표시용 근사 — 스파이크 한정.
 */
function reconstructFromUnified(diff: string): { original: string; modified: string } {
  const orig: string[] = []
  const mod: string[] = []
  for (const line of diff.split('\n')) {
    if (
      line.startsWith('diff ') ||
      line.startsWith('index ') ||
      line.startsWith('--- ') ||
      line.startsWith('+++ ') ||
      line.startsWith('@@')
    ) {
      continue
    }
    if (line.startsWith('+')) mod.push(line.slice(1))
    else if (line.startsWith('-')) orig.push(line.slice(1))
    else {
      const body = line.startsWith(' ') ? line.slice(1) : line
      orig.push(body)
      mod.push(body)
    }
  }
  return { original: orig.join('\n'), modified: mod.join('\n') }
}

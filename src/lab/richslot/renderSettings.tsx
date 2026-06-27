// 렌더 설정(표현 토글)을 레이아웃 트리로 흘리는 작은 컨텍스트.
//
// ★왜 컨텍스트★: codeRender/diffRender 는 ChatLayout → MarkdownView(펜스코드) 와
//  tool_result 펼침까지 여러 깊이로 내려가야 한다. prop-drilling 하면 모든 레이아웃
//  시그니처를 오염시키므로(레이아웃들은 messages 만 받는 계약 유지) 컨텍스트로 둔다.
//  설정값은 단순 enum 이라 컨텍스트 비용도 무시할 만하다.
//
// ★기본값★: 둘 다 lightweight(자체 렌더) — Monaco 는 사용자가 토글하기 전엔 안 뜬다.

import { createContext, useContext, type ReactNode } from 'react'

/** 펜스 코드 렌더: 'plain'=현 모노박스(무색) | 'monaco'=read-only Monaco(VS Code 강조). */
export type CodeRender = 'plain' | 'monaco'
/** diff 렌더: 'inline'=현 자체 +/- 색줄 | 'monaco'=Monaco diff 토큰 색. */
export type DiffRender = 'inline' | 'monaco'

export interface RenderSettings {
  codeRender: CodeRender
  diffRender: DiffRender
}

// 기본 = lightweight(Monaco 미로드). Provider 밖에서도 안전한 값.
const DEFAULT: RenderSettings = { codeRender: 'plain', diffRender: 'inline' }

const RenderSettingsContext = createContext<RenderSettings>(DEFAULT)

export function RenderSettingsProvider({
  value,
  children,
}: {
  value: RenderSettings
  children: ReactNode
}) {
  return <RenderSettingsContext.Provider value={value}>{children}</RenderSettingsContext.Provider>
}

export function useRenderSettings(): RenderSettings {
  return useContext(RenderSettingsContext)
}

/**
 * 텍스트가 unified diff 처럼 보이는지(no-dep 휴리스틱).
 * ★여기 두는 이유★: tool_result 본문이 diff 인지 판정해 Monaco diff 경로를 태울지 정하는데,
 *  Monaco 를 import 하는 MonacoCode.tsx 에 두면 layouts 가 그걸 import 하는 순간 monaco-editor
 *  청크가 eager 로 끌려와 lazy-load 가 깨진다. 그래서 무의존 모듈인 여기에 둔다.
 */
export function looksLikeDiff(text: string): boolean {
  const lines = text.split('\n')
  return (
    lines.some((l) => l.startsWith('diff --git') || l.startsWith('@@')) ||
    lines.filter((l) => /^[+-]/.test(l)).length >= 2
  )
}

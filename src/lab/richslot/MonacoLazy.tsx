// Monaco 컴포넌트의 lazy 경계 — 토글이 'monaco' 일 때만 청크를 가져온다.
//
// ★왜 분리★: MonacoCode.tsx 는 top-level 에서 `monaco-editor` 를 import 한다(무겁다).
//  여기서 React.lazy 로 감싸 dynamic import 하면, codeRender/diffRender 가 plain/inline 인
//  동안엔 monaco-editor 번들이 네트워크/평가되지 않는다 — 스파이크의 핵심(opt-in 비용).
//  렌더층(MarkdownView/ChatLayout)은 이 lazy 컴포넌트만 쓰고 Monaco 를 직접 모른다.

import { lazy, Suspense } from 'react'

const LazyCodeBlock = lazy(() =>
  import('./MonacoCode').then((m) => ({ default: m.MonacoCodeBlock })),
)
const LazyDiff = lazy(() => import('./MonacoCode').then((m) => ({ default: m.MonacoDiff })))

// Monaco 청크 로딩 중 자리표시(자체 모노박스 톤과 맞춤).
function Loading() {
  return <div className="md-code md-monaco-loading">loading editor…</div>
}

export function LazyMonacoCodeBlock(props: { code: string; lang: string }) {
  return (
    <Suspense fallback={<Loading />}>
      <LazyCodeBlock {...props} />
    </Suspense>
  )
}

export function LazyMonacoDiff(props: { diff: string }) {
  return (
    <Suspense fallback={<Loading />}>
      <LazyDiff {...props} />
    </Suspense>
  )
}

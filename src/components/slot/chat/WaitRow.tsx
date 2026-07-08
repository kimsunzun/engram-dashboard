// WaitRow — 응답 대기 인디케이터(스트림 끝 tail). ★임시/PROVISIONAL★: "Wait" + 애니메이션 점(…) + 마운트부터
//   올라가는 경과 초(0s → 1s → …). 사용자가 추후 정식 재설계 예정 — 지금은 최소 자족(self-contained) 구현이다.
//   구 "Thinking…" pulse 라벨을 이 컴포넌트가 대체한다(ADR-0051 rail tail 자리). 다음 세션은 이걸 갈아끼운다.
//
// ★타이머 정체성(load-bearing)★: 이 컴포넌트는 StructuredTextView 의 고정 key="__streaming__" ChatRow 안에
//   마운트된다 — 그 안정 key 덕에 스트리밍 리렌더 사이엔 remount 되지 않아 경과 초가 턴 도중 0 으로 리셋되지
//   않는다(턴 사이 full unmount→remount 에서만 리셋). setInterval 은 unmount 시 반드시 정리한다(누수·테스트
//   act 경고 방지).

import { useEffect, useState } from 'react'

export function WaitRow() {
  const [seconds, setSeconds] = useState(0)

  useEffect(() => {
    // 1초마다 경과 초 증가. cleanup 에서 clearInterval(누수·테스트 act 경고 방지 — 불변).
    const id = setInterval(() => setSeconds((s) => s + 1), 1000)
    return () => clearInterval(id)
  }, [])

  return (
    <div className="my-1 flex items-center gap-1 text-[13px] text-muted select-none">
      <span className="animate-pulse">Wait</span>
      {/* 애니메이션 점 — 진행 중 신호(은은한 pulse, ThoughtRow 와 동형). */}
      <span className="animate-pulse" aria-hidden>
        …
      </span>
      <span className="tabular-nums">{seconds}s</span>
    </div>
  )
}

export default WaitRow

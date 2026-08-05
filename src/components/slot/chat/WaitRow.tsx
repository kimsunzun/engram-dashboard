// ★임시/PROVISIONAL★: 사용자가 추후 정식 재설계 예정 — 지금은 최소 자족(self-contained) 구현이다.
//
// ★타이머 정체성(load-bearing)★: 이 컴포넌트는 StructuredTextView 의 고정 key="__streaming__" ChatRow 안에
//   마운트된다 — 그 안정 key 덕에 스트리밍 리렌더 사이엔 remount 되지 않아 경과 초가 턴 도중 0 으로 리셋되지
//   않는다(턴 사이 full unmount→remount 에서만 리셋).

import { useEffect, useState } from 'react'

export function WaitRow() {
  const [seconds, setSeconds] = useState(0)

  useEffect(() => {
    // cleanup 의 clearInterval 은 불변 — 누수·테스트 act 경고 방지.
    const id = setInterval(() => setSeconds((s) => s + 1), 1000)
    return () => clearInterval(id)
  }, [])

  return (
    <div className="my-1 flex items-center gap-1 text-[13px] text-muted select-none">
      <span className="animate-pulse">Wait</span>
      <span className="animate-pulse" aria-hidden>
        …
      </span>
      <span className="tabular-nums">{seconds}s</span>
    </div>
  )
}

export default WaitRow

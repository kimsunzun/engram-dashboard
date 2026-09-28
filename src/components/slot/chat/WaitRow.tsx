// ★임시/PROVISIONAL★: 사용자가 추후 정식 재설계 예정 — 지금은 최소 자족(self-contained) 구현이다.
//
// ★타이머 정체성(load-bearing)★: 이 컴포넌트는 StructuredTextView 의 고정 key="__streaming__" ChatRow 안에
//   마운트된다 — 그 안정 key 덕에 스트리밍 리렌더 사이엔 remount 되지 않아 경과 초가 턴 도중 0 으로 리셋되지
//   않는다(턴 사이 full unmount→remount 에서만 리셋).
// ★「중단하는 중」(ADR-0244)은 같은 인스턴스의 prop 이다★ — 다른 컴포넌트·key 로 갈라 그리면 끊기가 거절돼 대기
//   표시로 돌아올 때 경과 초가 0 이 된다. 그래서 그동안에도 타이머는 돈다(보이지만 않는다).

import { useEffect, useState } from 'react'
import { CircleStop } from 'lucide-react'

import { t } from '../../../i18n'

export function WaitRow({ interrupting = false }: { interrupting?: boolean }) {
  const [seconds, setSeconds] = useState(0)

  useEffect(() => {
    // cleanup 의 clearInterval 은 불변 — 누수·테스트 act 경고 방지.
    const id = setInterval(() => setSeconds((s) => s + 1), 1000)
    return () => clearInterval(id)
  }, [])

  if (interrupting) {
    // 결말 행(중단 — 굵게 · 강조색)과 같은 아이콘을 쓰되 톤은 대기 표시 그대로다 — 아직 멈춘 것이 아니다.
    return (
      <div
        data-wait-interrupting="1"
        className="my-1 flex items-center gap-1.5 text-[13px] text-muted select-none"
      >
        <CircleStop className="size-3.5 flex-none" aria-hidden />
        <span className="animate-pulse">{t('chat.interrupting')}</span>
      </div>
    )
  }

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

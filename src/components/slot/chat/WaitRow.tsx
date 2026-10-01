// ★임시/PROVISIONAL★: 사용자가 추후 정식 재설계 예정 — 지금은 최소 자족(self-contained) 구현이다.
//
// 사용자 결정 2026-10-01: 대기 표시는 대화 스크롤 목록 밖, 입력창 바로 위의 자기 줄(`WaitStrip`)에 선다. ★그 줄은 늘
//   마운트돼 있고 높이는 `--chat-wait-strip-h` 하나로 고정이다★ — 표시가 켜지고 꺼져도 대화 목록 높이가 그대로라 글이
//   튀지 않는다. 목록 끝 꼬리로 되돌리면 턴이 끝날 때마다 마지막 줄들이 표시 높이만큼 들썩인다.
// ★타이머 정체성(load-bearing)★: 경과 초는 `WaitRow` 인스턴스가 쥔다 — `streaming` 이 켜질 때 마운트돼 0 부터 세고
//   꺼질 때 사라진다. 줄 안 같은 자리에 같은 타입 하나뿐이라 그 사이 리렌더로는 다시 마운트되지 않는다.
// ★「중단하는 중」(ADR-0244)은 같은 인스턴스의 prop 이다★ — 다른 컴포넌트·key 로 갈라 그리면 끊기가 거절돼 대기
//   표시로 돌아올 때 경과 초가 0 이 된다. 그래서 그동안에도 타이머는 돈다(보이지만 않는다).

import { useEffect, useState, type ReactNode } from 'react'
import { CircleStop } from 'lucide-react'

import { t } from '../../../i18n'

// ADR-0263 — 입력창 위 고정 대기 줄(ADR-0244 구현 세부 b 개정).
/** @param label 오른쪽 칸 — 대기 글이 먼저 자리를 잡고 좁으면 이쪽이 줄어든다(넘기는 쪽이 `min-w-0 truncate` 를 단다). */
export function WaitStrip({
  streaming,
  interrupting,
  label,
}: {
  streaming: boolean
  interrupting: boolean
  label?: ReactNode
}) {
  return (
    <div
      data-wait-strip="1"
      className="flex flex-none items-center gap-2 overflow-hidden pr-3"
      // 왼쪽 = 대화 행 본문 들여쓰기(ChatRow px-4 + 레일 칸) — 레일 점·연결선은 그리지 않는다.
      style={{ height: 'var(--chat-wait-strip-h)', paddingLeft: 'calc(1rem + var(--chat-rail-gutter))' }}
    >
      {streaming && (
        <div className="flex-none whitespace-nowrap">
          <WaitRow interrupting={interrupting} />
        </div>
      )}
      {label !== undefined && <div className="ml-auto flex min-w-0 justify-end">{label}</div>}
    </div>
  )
}

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
        className="flex items-center gap-1.5 text-[13px] text-muted select-none"
      >
        <CircleStop className="size-3.5 flex-none" aria-hidden />
        <span className="animate-pulse">{t('chat.interrupting')}</span>
      </div>
    )
  }

  return (
    <div className="flex items-center gap-1 text-[13px] text-muted select-none">
      <span className="animate-pulse">Wait</span>
      <span className="animate-pulse" aria-hidden>
        …
      </span>
      <span className="tabular-nums">{seconds}s</span>
    </div>
  )
}

export default WaitRow

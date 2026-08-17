// 연결 실패 이유 알림(ADR-0134 결정 4).
//
// ★왜 화면에 그리나★: 데이터 폴더에 쓸 수 없어 데몬이 못 뜨는 실패는 릴리스에서 어디에도 도달하지
//   않았다 — 데몬은 콘솔이 없고, WebView2 릴리스 빌드엔 개발자도구가 없어 console.error 는 사용자가
//   볼 수 없다. 그래서 사용자는 원인 없는 'down' 만 봤다.
//
// ★새 제어 표면이 아니다★: 기존 연결 상태 표면(agentClient)을 **읽기만** 한다 — window 전역 핸들을
//   늘리지 않고 명령도 보내지 않는다(§5 LLM-우선 제어 — 제어 표면은 agentClient 하나).
//
// ★덮지 않는다(load-bearing)★: 이전 판은 `absolute top-0 z-50` 오버레이라 TabBar 를 가리고 그 클릭을
//   먹었다 — 탭 전환·생성·닫기가 막히고, 이유가 안 지워지는 상태와 겹치면 영구히 막힌다. 그래서
//   **일반 흐름 블록**으로 두고 호출부가 세로 스택으로 배치한다. 닫기 버튼은 그 위의 마지막 안전장치다.
//
// ★상시 크롬이 아니다(ADR-0063)★: 이유가 있을 때만 그린다. 없으면 null 이라 DOM 이 비고, ADR-0063 이
//   없앤 상시 StatusBar 를 되살리는 것이 아니다.

import { useEffect, useState } from 'react'

import { agentClient } from '../../api/clientFactory'

export default function ConnectionNotice() {
  const [reason, setReason] = useState<string | null>(agentClient.connectionError)
  // 닫은 이유 자체를 기억한다 — **다른** 이유가 오면 다시 뜬다(닫기가 영구 음소거가 되면 안 된다).
  const [dismissed, setDismissed] = useState<string | null>(null)

  useEffect(() => {
    // 상태 표면은 이유가 바뀔 때도 통지한다(ProtocolClient.reportConnectionError) — 그래서 별도
    // 구독 없이 이 하나로 충분하다.
    return agentClient.onConnectionStateChange(() => {
      const next = agentClient.connectionError
      setReason(next)
      // ★이유가 사라지면 닫힘 기억도 함께 버린다(load-bearing)★: 안 버리면 "실패 → 닫기 → 고침 →
      //   연결됨 → 나중에 **똑같은** 문구로 재발" 에서 문자열이 같아 아무것도 안 뜬다. 같은 원인이
      //   반복되는 것이 오히려 흔한 경우라, 닫기가 그 원인을 영구 음소거하면 안 된다.
      if (next === null) setDismissed(null)
    })
  }, [])

  if (!reason || reason === dismissed) return null

  return (
    <div
      role="alert"
      className="flex shrink-0 items-start gap-3 border-b border-destructive/40 bg-destructive/15 px-4 py-2 text-sm text-foreground"
    >
      <div className="min-w-0 flex-1">
        <span className="font-semibold">데몬에 연결하지 못했습니다.</span>{' '}
        <span className="opacity-90">{reason}</span>
      </div>
      <button
        type="button"
        aria-label="알림 닫기"
        className="shrink-0 rounded px-2 opacity-70 hover:opacity-100"
        onClick={() => setDismissed(reason)}
      >
        ✕
      </button>
    </div>
  )
}

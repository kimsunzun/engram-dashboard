// SlotUnavailableVeil — "타겟한 에이전트가 지금 없다" 를 세 슬롯(터미널·DOM·리치)이 **한 벌로** 그리는 막.
//
// ★내용은 지우지 않는다(ADR-0148 결정 2·3, 사용자 결정)★: 대화든 터미널 화면이든 그 시점 모습을 그대로
//   두고, 반투명 막으로 덮어 흐리게 만든 뒤 가운데에 심볼 하나만 얹는다. 화면을 비우면 "돌아오면 이어서
//   볼 것" 이 사라진다. 부재를 세 갈래로 가르는 상위 판정은 ADR-0149.
//
// ★문구를 넣지 않는다(사용자 결정 2026-08-20)★: 옛 터미널·DOM 슬롯은 대기/종료/`Failed: <메시지>` 를
//   글로 적었는데, 그러면 같은 사실을 슬롯마다 다른 문장으로 말하게 되고 데몬 재기동처럼 판정이 흔들리는
//   구간에서 "종료됨" 같은 단정이 그대로 남는다. 심볼 하나면 그 단정을 하지 않는다. 실패 사유가 필요하다는
//   것이 드러나면 그때 되살린다 — 지금 미리 자리를 만들지 않는다.
//
// ★컴포넌트 하나로 두는 이유★: 세 슬롯이 각자 그리면 "같은 조건에 같은 모습" 이 규약으로만 남는다.
//   조건(부재 판정)은 각 슬롯이 자기 데이터로 계산하고, 그 결과를 이 한 자리가 그린다.

import { PowerOff } from 'lucide-react'

import type { ViewPhase } from '../../api/agentClient'

interface SlotUnavailableVeilProps {
  /**
   * 구독이 마지막으로 알린 국면(아직 아무 통지도 못 받았으면 생략). 그리는 모습은 이 값으로 바뀌지 않고
   * **관측 표면(cdp·테스트)에서 부재의 근거를 가르는 데만** 쓴다 — 대기(`detached`)와 재요청 소진
   * (`error`)은 사용자에게 같은 그림이지만 자동화에는 다른 사실이다.
   */
  phase?: ViewPhase | null
}

export function SlotUnavailableVeil({ phase }: SlotUnavailableVeilProps) {
  return (
    <div
      data-slot-dead="1"
      data-slot-phase={phase ?? undefined}
      // 옛 관측 표면 유지 — "붙을 에이전트를 기다리는 중" 만 참(`error` 는 아니다).
      data-slot-detached={phase === 'detached' ? '1' : undefined}
      aria-hidden
      // pointer-events-none — 아래 조작을 막지 않는다(입력 비활성은 각 슬롯의 disabled 가 담당).
      className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center bg-background/70"
    >
      <PowerOff className="size-10 text-muted" />
    </div>
  )
}

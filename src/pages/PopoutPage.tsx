// PopoutPage — 슬롯 팝업 분리(pop-out)·빈 창 생성(create_window)으로 런타임 생성된 OS 창이 그리는 페이지.
//
// ★탭 소유 모델(ADR-0057)★: 옛 "고정 단일 View" 팝업이 아니라 **탭 가진 일반 창**이다(D-2 "팝업도 탭").
//   URL 키 `#/popup?window=<label>` 로 자기 창 label 을 확정하고, WindowLayout(label) 을 얇게 감싼다 —
//   탭바·슬롯 캔버스·탭 상태 학습·0탭 자가닫힘은 전부 그 컴포넌트가 소유한다(§7-1). PopoutPage 는
//   label 만 뽑아 넘기는 껍데기다.
//
// ★view:closed 은퇴(G2)★: 옛 "자기 view 가 닫히면 창 자가종료"(view:closed 리스너)는 제거됐다. 창 닫힘은
//   백엔드 close_window 단일 소스(§5-2)이고, 프론트 자가닫힘은 WindowLayout 의 window:tabs-updated{tabs:[]}
//   (0탭) 신호로만 일어난다(이중 발화·재진입 방지). 이 페이지엔 자가종료 로직이 없다.
//
// ★출력 구독은 자동★: 이 창은 자기 TauriTransport 싱글톤(별 WebView2 프로세스라 모듈 그래프가 신선)이
//   connected 전이에서 subscribe_output 을 자기 window_label 로 등록한다 → 이 창 tabs 의 agent 출력이
//   이 창 Channel 로 라우팅된다(백엔드 OutputRouter 일반 메커니즘, 모든 탭 walk). 슬롯은 메인과 동일하게
//   agentClient 로 구독하고 request_replay(gen 펜스, ADR-0046)로 replay 를 받는다 — 팝업 전용 배선 없음.
//
// ★§5★: 이 창의 생성/바인딩 자체가 command(`slot.popout`·`window.create` = invoke)로 LLM 제어 가능하다.
//   이 페이지는 순수 I/O 표시 표면(손발) — 제어는 백엔드측(두뇌)이 쥔다.

import { useState } from 'react'

import ConnectionNotice from '../components/layout/ConnectionNotice'
import WindowLayout from '../components/layout/WindowLayout'
// ★단일 출처★: 이 창 label 파싱은 viewStore 의 공유 헬퍼를 쓴다(WindowLayout·useCurrentViewId·
//   SlotContextMenu 가 같은 판정을 공유 — §5 제어 표면 일관).
import { readWindowLabelFromHash } from '../store/viewStore'

export default function PopoutPage() {
  // URL `?window=` 에서 1회 확정 — 창 수명 동안 불변. 팝업 라우트가 아니면 'main' 폴백이나,
  // /popup 라우트에서만 이 페이지가 뜨므로 정상 경로는 팝업 label 이다.
  const [label] = useState<string>(readWindowLabelFromHash)

  return (
    // 알림은 모든 창에 나온다 — 부팅은 창마다 돌고 실패 이유도 창마다 도착한다(ADR-0134).
    <div
      style={{
        width: '100vw',
        height: '100vh',
        background: 'var(--bg)',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      <ConnectionNotice />
      <div style={{ flex: 1, minHeight: 0 }}>
        <WindowLayout label={label} />
      </div>
    </div>
  )
}

import { useEffect, useSyncExternalStore } from 'react'

import ConnectionNotice from './ConnectionNotice'
import NoticeOverlay from './NoticeOverlay'
import RestoreModal from './RestoreModal'
import StateFileNotice from './StateFileNotice'
import WindowLayout from './WindowLayout'
import { restoreClient, type RestoreClient } from '../../api/restoreClient'
import { MAIN_WINDOW_LABEL } from '../../store/viewStore'

interface Props {
  /** 시험이 가짜를 꽂는 자리(ADR-0012) — 운영은 기본값. */
  restore?: RestoreClient
}

// ★고정 크롬 없음(ADR-0063)★: 좌측 고정 Sidebar(AgentList 마운트)·하단 DiffPanel·StatusBar 를 두지 않는다.
//   에이전트 트리는 슬롯 콘텐츠로만 존재하고, 부팅엔 없다 — 빈 슬롯 메뉴나 `layout.setSlotContent` 로
//   놓는다(ADR-0222). 슬롯이라 이동·분할·닫기·재배치 가능하다(§5 LLM 제어 표면). StatusBar/DiffPanel 은
//   S0 뷰-단계 잔재(실기능 0)였다 — 진짜 diff/상태바가 필요하면 재구현한다.
//
// ★단일 레이아웃 권위(ADR-0035·0057)★: 메인 캔버스·슬롯 우클릭 메뉴(SlotContextMenu)·트리 배정 모두
//   viewStore(=백엔드 ViewManager 미러)로 단일화. main·팝업이 같은 WindowLayout 을 마운트해 동일 코드경로(D-2).
//
// ★복원 모달 · 상태 파일 알림은 여기서만 단다★ — 이 라우트가 main 이다. `App` 은 창마다 돌아 거기 달면 팝아웃까지
//   막힌다(TRD S21-storage §6-7 · 트리 창은 ADR-0225 로 걷혔다).
//   둘은 같은 상태 한 벌(`restoreClient` — 구독 하나 · 당기기 하나)을 읽는다.
export default function AppLayout({ restore = restoreClient }: Props) {
  useEffect(() => restore.install(), [restore])
  const status = useSyncExternalStore(restore.subscribe, restore.status)
  const awaiting = status?.crash_copy === 'awaiting'

  return (
    <>
      {/* 알림도 잠기는 층 안이다 — 묻는 동안 알림의 ✕ 도 막힌다. */}
      <div inert={awaiting} style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ flex: 1, minHeight: 0, position: 'relative' }}>
          <NoticeOverlay
            // ADR-0276: 상태 파일 알림은 레이아웃을 밀지 않고 덮는다(거부한 대안 = 미는 흐름 블록).
            // ADR-0277: 연결 띠도 이 층에서 덮는다(ADR-0180 개정).
          >
            {/* 연결 띠가 맨 위다 — 데몬에 못 붙으면 다른 아무것도 동작하지 않아 가장 먼저 읽혀야 한다(세션 판단). */}
            <ConnectionNotice />
            <StateFileNotice stateFile={status?.state_file} saves={status?.saves} />
          </NoticeOverlay>
          <WindowLayout label={MAIN_WINDOW_LABEL} />
        </div>
      </div>
      {/* 덮는 층은 묻는 동안에만 그린다 — 늘 깔아 두면 투명해도 그 아래 클릭을 먹는다. */}
      {awaiting && <RestoreModal status={status} onAnswer={restore.answer} />}
    </>
  )
}

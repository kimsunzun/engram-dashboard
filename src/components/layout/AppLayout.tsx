import ConnectionNotice from './ConnectionNotice'
import WindowLayout from './WindowLayout'
import { MAIN_WINDOW_LABEL } from '../../store/viewStore'

// ★고정 크롬 없음(ADR-0063)★: 좌측 고정 Sidebar(AgentList 마운트)·하단 DiffPanel·StatusBar 를 두지 않는다.
//   에이전트 트리는 슬롯 콘텐츠로만 존재하고, 부팅엔 없다 — 빈 슬롯 메뉴나 `layout.setSlotContent` 로
//   놓는다(ADR-0222). 슬롯이라 이동·분할·닫기·재배치 가능하다(§5 LLM 제어 표면). StatusBar/DiffPanel 은
//   S0 뷰-단계 잔재(실기능 0)였다 — 진짜 diff/상태바가 필요하면 재구현한다.
//
// ★단일 레이아웃 권위(ADR-0035·0057)★: 메인 캔버스·슬롯 우클릭 메뉴(SlotContextMenu)·트리 배정 모두
//   viewStore(=백엔드 ViewManager 미러)로 단일화. main·팝업이 같은 WindowLayout 을 마운트해 동일 코드경로(D-2).
export default function AppLayout() {
  return (
    // 세로 스택 — 알림은 덮지 않고 밀어낸다(오버레이면 TabBar 클릭을 먹는다, ConnectionNotice 주석).
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <ConnectionNotice />
      <div style={{ flex: 1, minHeight: 0 }}>
        <WindowLayout label={MAIN_WINDOW_LABEL} />
      </div>
    </div>
  )
}

import WindowLayout from './WindowLayout'
import { MAIN_WINDOW_LABEL } from '../../store/viewStore'

// AppLayout — main 창의 얇은 셸. 슬롯 영역(탭바 + 활성 탭 캔버스)은 WindowLayout("main")이 소유한다.
//
// ★고정 크롬 제거(ADR-0063)★: 옛 좌측 고정 Sidebar(AgentList 마운트)·하단 더미 DiffPanel·StatusBar 를
//   전부 제거했다. 에이전트 트리는 이제 부팅 기본 레이아웃의 agent_list 슬롯(ViewManager::new)으로만 뜨고,
//   슬롯이라 이동·분할·닫기·재배치(set_slot_content) 가능하다(§5 LLM 제어 표면). StatusBar/DiffPanel 은
//   S0 뷰-단계 잔재(실기능 0)라 삭제 — 진짜 diff/상태바가 필요하면 재구현한다.
//
// ★단일 레이아웃 권위(ADR-0035·0057)★: 메인 캔버스·슬롯 우클릭 메뉴(SlotContextMenu)·트리 배정 모두
//   viewStore(=백엔드 ViewManager 미러)로 단일화. main·팝업이 같은 WindowLayout 을 마운트해 동일 코드경로(D-2).
export default function AppLayout() {
  return (
    <div style={{ height: '100%' }}>
      <WindowLayout label={MAIN_WINDOW_LABEL} />
    </div>
  )
}

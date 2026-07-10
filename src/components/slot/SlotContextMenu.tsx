import { useEffect, useRef } from 'react'
import { useCurrentViewId, useViewStore } from '../../store/viewStore'
import { useAgentStore } from '../../store/agentStore'
import { agentClient } from '../../api/clientFactory'
import type { SlotContent, SplitDir } from '../../api/layoutTypes'

interface SlotContextMenuProps {
  x: number
  y: number
  /** 대상 슬롯 id — 백엔드 권위 레이아웃의 string UUID(ADR-0035). */
  slotId: string
  /** 이 슬롯에 배정된 에이전트(있으면 종료 메뉴 활성). 렌더러가 wire LayoutNode.agent_id 로 넘긴다. */
  agentId?: string | null
  /**
   * ★이 메뉴가 조작할 View id 오버라이드(선택).★ WindowLayout 이 각 탭 캔버스에 그 탭 view 를 내려꽂아
   * (§3-4/G7) 분할/닫기/pop-out 액션이 이 탭 좌표를 쓰게 한다 — 이 컴포넌트는 "어느 창인지" 무지로 남는다.
   * 값이 없으면(직접 마운트 등) 이 웹뷰 창의 active 탭(useCurrentViewId)으로 폴백한다(엉뚱한 view 방지).
   */
  viewIdOverride?: string | null
  onClose: () => void
}

// ★§5 손발/두뇌 분리 — 사람 우클릭도 LLM(window.__engramLayout)과 같은 단일 제어 표면을 흔든다★:
// 이 메뉴의 레이아웃 액션(분할/닫기/배정)은 viewStore 액션(split/closeSlot/assignAgent)만 부른다.
// 그 액션들은 각기 대응 invoke 를 쳐 백엔드 ViewManager(권위)를 바꾸고, emit→listen 루프로 화면이 갱신된다
// (낙관적 갱신 X). 옛 slotStore.dispatch(프론트 전용 권위)는 제거됐다(Brick 1).
export default function SlotContextMenu({ x, y, slotId, agentId, viewIdOverride, onClose }: SlotContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null)
  // 이 웹뷰 창의 active 탭 id — 레이아웃 mutation 은 (viewId, slotId) 쌍으로 지정한다(백엔드 권위 좌표계).
  const currentViewId = useCurrentViewId()
  // ★이 메뉴가 실제로 조작할 View 좌표★ — WindowLayout 이 넘긴 탭 오버라이드가 있으면 그걸, 없으면 이
  //   웹뷰 창의 active 탭(§3-4). 아래 모든 레이아웃 액션(spawn→assign·split·closeSlot·move)이 이 값을 쓴다.
  const targetViewId = viewIdOverride ?? currentViewId
  // 레이아웃 액션(viewStore) — 단일 제어 표면. window.__engramLayout 이 노출하는 것과 동일 함수.
  const split = useViewStore(s => s.split)
  const closeSlot = useViewStore(s => s.closeSlot)
  const assignAgent = useViewStore(s => s.assignAgent)
  const moveSlotToWindow = useViewStore(s => s.moveSlotToWindow)
  const setSlotContent = useViewStore(s => s.setSlotContent)
  const agents = useAgentStore(s => s.agents)

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [onClose])

  const slotAgentId = agentId ?? null

  const items = [
    {
      label: '에이전트 생성',
      action: () => {
        // spawn 은 에이전트 명령(agentClient/ProtocolClient seam, ADR-0011) — 데몬 권위.
        // 성공하면 그 agentId 를 이 슬롯에 배정(레이아웃 명령, ADR-0035)한다 — 두 권위가 분리돼 있다.
        if (!targetViewId) {
          console.warn('[SlotContextMenu] view id 없음 — 배정 대상 뷰 미확정')
          return
        }
        const cwd = window.prompt('작업 디렉토리', 'C:/') ?? ''
        if (!cwd.trim()) return
        agentClient
          .spawnAgent(cwd.trim())
          // ADR-0035: 배정도 백엔드 권위 invoke(assign_agent) — 프론트 낙관 갱신 없이 emit 으로 반영.
          .then(agent => assignAgent(targetViewId, slotId, agent.id))
          .catch(e => console.error('[spawn]', e))
      },
    },
    {
      label: '에이전트 종료',
      action: () => {
        if (!slotAgentId) return
        // 종료도 에이전트 명령(ADR-0011). 슬롯은 그대로 두고(레이아웃 불변) 에이전트만 kill 한다.
        agentClient.killAgent(slotAgentId).catch(e => console.error('[kill]', e))
      },
    },
    {
      label: '팝업으로 분리',
      action: () => {
        // ADR-0057: 슬롯 agent 를 새 런타임 팝업 창의 새 탭으로 MOVE(detach) — viewStore.moveSlotToWindow →
        //   invoke(move_slot_to_window, toWindow=null) → 백엔드가 새 View 생성·이전·새 팝업 창·탭 삽입·원본
        //   슬롯 제거 후 emit. §5: window.__engramLayout.moveSlotToWindow 와 동일 표면(사람 클릭 = LLM 한 표면).
        if (!targetViewId) return console.warn('[SlotContextMenu] view id 없음 — move 무시')
        if (!slotAgentId) return
        void moveSlotToWindow(targetViewId, slotId).catch(e => console.error('[moveSlotToWindow]', e))
      },
    },
    // ★슬롯 콘텐츠 배치(ADR-0063)★: 옛 갭("트리/터미널 보기는 백엔드 wire 없음")이 set_slot_content 제네릭
    //   command 로 해소됐다. 이제 빈 슬롯(또는 어느 슬롯)에 트리(agent_list)·팔레트(preset_palette)를
    //   배치하거나 비운다(empty) — viewStore.setSlotContent → invoke(set_slot_content) → emit 반영(§5
    //   단일 제어 표면, window.__engramLayout.setSlotContent 와 동일 함수). ADR-0060: SlotContent 유니온
    //   discriminated 태그로 넘긴다.
    { label: '에이전트 트리 열기', action: () => dispatchSetContent({ type: 'agent_list' }) },
    { label: '프리셋 팔레트 열기', action: () => dispatchSetContent({ type: 'preset_palette' }) },
    { label: '비우기', action: () => dispatchSetContent({ type: 'empty' }) },
    // ADR-0035: 분할 = viewStore.split(targetViewId, slotId, dir) → invoke(split_slot) → emit 반영.
    { label: '가로 분할', action: () => dispatchSplit('horizontal') },
    { label: '세로 분할', action: () => dispatchSplit('vertical') },
    // ADR-0035: 닫기 = viewStore.closeSlot(targetViewId, slotId) → invoke(close_slot)(형제 승격).
    { label: '닫기', action: () => dispatchCloseSlot() },
  ]

  function dispatchSplit(dir: SplitDir): void {
    if (!targetViewId) return console.warn('[SlotContextMenu] view id 없음 — split 무시')
    void split(targetViewId, slotId, dir).catch(e => console.error('[split]', e))
  }

  function dispatchCloseSlot(): void {
    if (!targetViewId) return console.warn('[SlotContextMenu] view id 없음 — closeSlot 무시')
    void closeSlot(targetViewId, slotId).catch(e => console.error('[closeSlot]', e))
  }

  // ADR-0063: 슬롯 콘텐츠 배치 = viewStore.setSlotContent(targetViewId, slotId, content) →
  //   invoke(set_slot_content) → emit 반영(백엔드 권위, 낙관 갱신 X).
  function dispatchSetContent(content: SlotContent): void {
    if (!targetViewId) return console.warn('[SlotContextMenu] view id 없음 — setSlotContent 무시')
    void setSlotContent(targetViewId, slotId, content).catch(e => console.error('[setSlotContent]', e))
  }

  // agent-필요 항목('에이전트 종료'·'팝업으로 분리')은 슬롯에 실행중 에이전트가 있을 때만 유효 — 없으면
  // 흐리게(클릭 무해 no-op). 그 외 항목은 항상 활성. (Brick 1 enabled 가드 패턴 재사용 — 대상 라벨만 확장.)
  const AGENT_REQUIRED_LABELS = ['에이전트 종료', '팝업으로 분리']
  const hasLiveAgent = slotAgentId != null && agents.some(a => a.id === slotAgentId)
  const isKillable = (label: string): boolean =>
    !AGENT_REQUIRED_LABELS.includes(label) || hasLiveAgent

  return (
    <div ref={ref} style={{
      position: 'fixed',
      top: y,
      left: x,
      background: 'var(--bg-secondary)',
      border: '1px solid var(--border)',
      borderRadius: '4px',
      zIndex: 1000,
      minWidth: '130px',
      boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
      fontFamily: 'var(--font-ui)',
      fontSize: '12px',
    }}>
      {items.map(item => {
        const enabled = isKillable(item.label)
        return (
          <div
            key={item.label}
            style={{ padding: '6px 12px', cursor: 'pointer', color: enabled ? 'var(--text)' : 'var(--text-muted)' }}
            onMouseEnter={e => (e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)')}
            onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
            // 비활성 항목은 클릭해도 무해(no-op) — action 을 실행하지 않고 메뉴만 닫는다.
            // enabled 기본값은 '활성'(isKillable 은 '에이전트 종료' 외 모든 항목에 true) → 명시적 false 일 때만 차단.
            onClick={e => {
              e.stopPropagation()
              if (enabled) item.action()
              onClose()
            }}
          >{item.label}</div>
        )
      })}
    </div>
  )
}

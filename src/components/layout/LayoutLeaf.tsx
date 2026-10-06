// ADR-0227: 슬롯 하나를 그리는 잎 — 평평한 렌더러(ViewLayoutRenderer)가 루트 하나 아래에 slot id 를 key 로
//   나란히 그린다. 분할·닫기·승격에도 부모·key 가 바뀌지 않아 잎과 그 안의 슬롯별 기억·터미널이 살아남는다.
//   팝아웃은 새 slot id·새 웹뷰라 새로 마운트된다.

import { memo, useCallback, useEffect, useRef, useState, type CSSProperties } from 'react'
import { Plus } from 'lucide-react'

import type { LayoutNode, SlotRect } from '../../api/layoutTypes'
import { useCurrentViewId, useViewStore } from '../../store/viewStore'
import { useAgentStore } from '../../store/agentStore'
import TerminalSlot from '../slot/TerminalSlot'
import RichSlot from '../slot/RichSlot'
import DomSlot from '../slot/DomSlot'
import PresetPalette from '../slot/PresetPalette'
import UsageSlot from '../slot/UsageSlot'
import AgentList from '../agent/AgentList'
import { isContentSlot } from '../agent/selectOpenTarget'
import { agentPresence } from '../agent/mergeTreeNodes'
import SlotContextMenu from '../slot/SlotContextMenu'
import { SlotErrorBoundary } from '../slot/SlotErrorBoundary'
import { SlotUnavailableVeil } from '../slot/SlotUnavailableVeil'
import { buildSlotMenu } from '../../commands/slotMenu'
import { defaultRenderMode, type RenderMode } from '../slot/renderMode'
import { t } from '../../i18n'
import { reportUiMetrics } from './uiMetricsReport'

type SlotNode = Extract<LayoutNode, { type: 'slot' }>
type MenuAnchor = { x: number; y: number }
/** 에이전트 슬롯의 표시 — 판정은 SlotBody 의 ADR-0149 블록. `view` = 구체 렌더러(살아 있거나 기억으로 지키는 뷰). */
type AgentSlotState = 'view' | 'connecting' | 'stopped' | 'noTarget'

// memo: 구분선 미리보기 중 사각형이 안 바뀐 잎은 틀도 다시 그리지 않는다 — 재사상(`remapRects`)이 그 사각형 객체를
//   그대로 돌려준다. 사각형이 바뀐 잎도 다시 그리는 것은 틀뿐이다(아래 SlotBody). 스토어 구독은 memo 와 무관하게
//   잎을 갱신한다.
export default memo(LayoutLeaf)

function LayoutLeaf({
  node,
  rect,
  focusedSlotId,
  isForeign,
  viewIdOverride,
}: {
  node: SlotNode
  /** 셸이 계산한 이 칸의 사각형(뷰 기준 정규화) — 미리보기 재사상이 입혀졌을 수 있다. */
  rect: SlotRect
  focusedSlotId: string | null
  /** 이 슬롯이 스냅샷 `foreign_slots` 에 있다 — 이 판이 모르는 내용을 쥐어 `empty` 로 실렸지만 비어 있지 않다. */
  isForeign: boolean
  viewIdOverride?: string | null
}) {
  // ★우클릭 슬롯 메뉴 상태(§5)★: 잎은 슬롯 하나당 하나라 여기 useState 는 그 슬롯 전용 메뉴 좌표다.
  //   열림 시 SlotContextMenu 를 이 잎 안에서 직접 마운트한다.
  const [contextMenu, setContextMenu] = useState<MenuAnchor | null>(null)
  // 참조가 고정돼야 SlotBody 의 memo 가 선다.
  const closeMenu = useCallback(() => setContextMenu(null), [])
  // ★이 메뉴가 조작할 View 좌표(ADR-0064)★: 옛 SlotContextMenu 내부 폴백을 여기(ctx 조립처)로 끌어올렸다.
  const currentViewId = useCurrentViewId()
  const targetViewId = viewIdOverride ?? currentViewId

  // ★틀과 테두리를 가른다(ADR-0227)★: 바깥 틀 = 셸 사각형 그대로 + overflow:hidden, 테두리 없음. 위치·크기
  //   규칙은 index.css 의 `.engram-slot-frame` 이 이 사용자 속성 넷으로 계산한다(반올림 포함). 테두리를 틀에
  //   되돌리면 0~1px 칸에서 테두리가 할당 폭을 넘어 이웃과 겹친다 — 안쪽 요소는 틀에 잘린다.
  //   ★transform·contain 을 걸지 말 것★ — SlotContextMenu 가 position:fixed 라 조상의 transform 이 좌표계를 깬다.
  const frameStyle: CSSProperties & Record<`--${string}`, number> = {
    '--x0': rect.x0,
    '--y0': rect.y0,
    '--x1': rect.x1,
    '--y1': rect.y1,
    position: 'absolute',
    overflow: 'hidden',
  }
  return (
    <div
      className="engram-slot-frame"
      style={frameStyle}
      // 슬롯 식별용 data 속성 — cdp eval 에서 DOM 으로 split 결과(슬롯 수)를 셀 수 있게. 이벤트와 함께 틀에 둔다:
      //   cdp 레시피와 테스트가 [data-slot-id] 에 이벤트를 직접 쏘고, 안쪽 클릭은 버블로 여기 닿는다.
      data-slot-id={node.id}
      // ADR-0066: click-to-focus — 슬롯 pane 클릭 시 이 슬롯을 포커스로 지정한다. viewStore.focusSlot →
      //   invoke(focus_slot) → emit(layout:updated) 단일 제어 표면(사람 클릭 = LLM = slot.focus command, §5).
      //   ★낙관 갱신 X★: 링(isFocused)은 백엔드 emit 스냅샷으로만 갱신된다(권위 = src-tauri, ADR-0035).
      //   ★버블 허용(stopPropagation/preventDefault 안 함)★: 내부 상호작용(터미널 포커스·AgentList 버튼 등)을
      //   가로채지 않는다 — pane 어디를 눌러도 내부 핸들러가 그대로 발화한다.
      //   ★제어 슬롯 포커스 제외 — allowlist(콘텐츠 슬롯만 포커스), ADR-0066 정제★: 콘텐츠 슬롯
      //   (empty/agent)일 때만 focusSlot 호출한다. 트리(agent_list)·팔레트(preset_palette) 등 제어 슬롯,
      //   그리고 앞으로 추가될 제어 variant(ADR-0060 FileTree/ControlPanel 등)는 자동으로 비포커스된다
      //   — 게이트 기준을 denylist(제어 나열)가 아니라 selectOpenTarget 와 공유하는 단일 분류기
      //   isContentSlot(allowlist)로 잡아 "열기" 대상 선택과 기준 이원화를 막는다.
      //   이 게이트가 없으면 트리 노드 좌클릭이 트리 슬롯 pane 까지 버블해 트리 슬롯이 포커스되고, 이어
      //   우클릭 "열기"가 그 트리 슬롯을 대상으로 잡아 트리를 에이전트 터미널로 덮어썼다(선존 UX 버그).
      //   targetViewId 미확정(부팅 직후 탭 상태 미도착)이면 no-op(잘못된 view 로 focus 유출 방지).
      // ADR-0144: 빈 슬롯 좌클릭은 메뉴를 열지 않는다(포커스만) — 매 클릭마다 메뉴가 뜨는 게 불편하다는
      //   제보로 ADR-0141/0143 의 좌클릭 오프너를 되돌렸다. 메뉴는 우클릭 전용(아래 onContextMenu).
      onClick={() => {
        if (!isContentSlot(node.content)) return
        if (targetViewId) void useViewStore.getState().focusSlot(targetViewId, node.id)
      }}
      // ADR-0064: 메뉴 항목은 command id 로만 실행한다 — 메뉴가 store 를 직접 부르지 않는다.
      //   그래서 사람 우클릭과 LLM 이 같은 진입점을 지난다(§5).
      onContextMenu={e => {
        e.preventDefault()
        setContextMenu({ x: e.clientX, y: e.clientY })
      }}
    >
      <SlotBody
        node={node}
        isFocused={node.id === focusedSlotId}
        isForeign={isForeign}
        targetViewId={targetViewId}
        contextMenu={contextMenu}
        onCloseMenu={closeMenu}
      />
    </div>
  )
}

// ★사각형을 받지 않는다★: 구분선을 끄는 동안 프레임마다 바뀌는 것은 틀의 사용자 속성 넷뿐이다. 이 몸이 사각형을
//   받으면 그때마다 슬롯 렌더러(터미널·마크다운 재파싱)까지 다시 그린다.
const SlotBody = memo(function SlotBody({
  node,
  isFocused,
  isForeign,
  targetViewId,
  contextMenu,
  onCloseMenu,
}: {
  node: SlotNode
  isFocused: boolean
  isForeign: boolean
  targetViewId: string | null
  contextMenu: MenuAnchor | null
  onCloseMenu: () => void
}) {
  const renderModeOverride = useViewStore(s => s.renderModeOverride)
  // ★M2 caps 분기(ADR-0044)★: agent 배정 슬롯의 렌더러는 그 agent 의 output caps 로 고른다. caps 는
  // AgentInfo 로 이미 wire 를 건너와 store 에 있다(M1) — 여기선 조회만(추가 배선 불필요).
  const agents = useAgentStore(s => s.agents)
  // ADR-0148: 종료로 명부에서 수거된 에이전트를 "아직 명부를 못 받음" 과 갈라야 한다 — 그 판별 신호들.
  const agentsLoaded = useAgentStore(s => s.agentsLoaded)
  // profiles 는 실 store 에선 항상 배열이지만 일부 단위테스트 mock 이 안 채운다 → 방어적 기본값(RichSlot 동형).
  const profiles = useAgentStore(s => s.profiles) ?? []
  const profilesLoaded = useAgentStore(s => s.profilesLoaded) ?? false
  // ★이 슬롯이 마지막으로 마운트한 렌더 대상★(ADR-0148): 에이전트가 명부에서 사라진 뒤에도 같은 컴포넌트를
  //   계속 렌더해야 화면 내용이 남는다 — 그런데 모드는 caps(AgentInfo)에서 유도되므로 그때는 다시 구할 수
  //   없다. 잎은 slot id 로 키잉돼 슬롯 하나당 하나라 이 ref 가 그 슬롯의 기억이다.
  //   ★데몬은 종료 시 replay ring 을 세션과 함께 버린다★ — 뷰를 내리면 그 대화는 재구독으로도 못 살린다.
  //   그래서 이 기억은 편의가 아니라 데이터 보존 수단이다.
  //   ★agentId 로 키잉하고 어긋나면 버린다★: 슬롯에 다른 에이전트를 배정하면 이 기억은 죽는다(아래 effect).
  //   무시만 하고 남겨 두면 그 에이전트를 다시 배정할 때 기억이 되살아나, 이미 언마운트된 빈 뷰를 흐림
  //   상태로 다시 마운트하고 수거된 에이전트로 구독까지 건다(기억 없는 정지 슬롯은 슬롯 컴포넌트를 띄우지 않고
  //   막만 그린다 — 아래 판정).
  //   ★담는 것은 렌더 모드 하나뿐이다(ADR-0149 결정 5)★ — 회차 번호(epoch)는 여기도 슬롯 prop 에도 없다.
  //   슬롯의 재구독 트리거에서 화신을 뺐기 때문이다(비우기는 구독의 onReset 단독 — 각 슬롯 주석).
  //   ★알려진 한계★: 부재 구간에는 mode 유도가 없어 setRenderMode/clearRenderMode 가 조용히 무효다(호출은
  //   성공으로 보인다).
  const lastMountRef = useRef<{ agentId: string; mode: RenderMode } | null>(null)

  // 칸 틀 기본 지표(TRD §2d)는 이 잎의 테두리 요소에서 잰다 — 셸의 칸 content = 틀 − 이 테두리 폭.
  //   웹뷰당 한 번만 나간다(성공 뒤로는 즉시 돌아온다).
  const borderRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (borderRef.current !== null) void reportUiMetrics(borderRef.current)
  }, [])

  // ADR-0060: 슬롯 점유자 = SlotContent 태그드 유니온.
  const slotAgentId = node.content.type === 'agent' ? node.content.agent_id : null
  const agent = slotAgentId != null ? (agents.find(a => a.id === slotAgentId) ?? null) : null
  const mode = agent != null ? (renderModeOverride[node.id] ?? defaultRenderMode(agent)) : null

  // ★기억 쓰기는 커밋 후에만(effect)★: 렌더 중에 쓰면 **폐기되는 concurrent 렌더**가 커밋되지 않은
  //   mode 를 남길 수 있고, 그 뒤 에이전트가 사라지면 그 오염된 신원으로 자식을 갈아 마운트해 커밋된
  //   대화를 지운다. effect 로 옮기면 "실제로 마운트된 사실"만 기록된다.
  // ★그리고 어긋난 기억은 여기서 버린다(G2)★: 배정이 이 기억의 에이전트가 아니게 된 순간 지운다 —
  //   남겨 두면 같은 에이전트를 다시 배정할 때 이미 언마운트된 빈 뷰가 흐림 상태로 되살아난다.
  // deps 는 원시값만 — agent 객체는 store 갱신마다 신원이 바뀌어 매번 재실행된다(쓰는 값은 같아 무해하지만
  //   불필요하다).
  useEffect(() => {
    if (agent != null && mode != null) {
      lastMountRef.current = { agentId: agent.id, mode }
      return
    }
    if (lastMountRef.current != null && lastMountRef.current.agentId !== slotAgentId) {
      lastMountRef.current = null
    }
  }, [agent?.id, mode, slotAgentId]) // eslint-disable-line react-hooks/exhaustive-deps

  // ★caps 도착 후에만 구체 렌더러를 마운트한다(ADR-0041 replay 소유권)★: 데몬 replay 는 slot-assign
  //   델타((window,agent) 키)에서 단 1회만 발화하고, 컴포넌트 스왑(TerminalSlot→RichSlot)엔 재발화하지
  //   않는다. 그래서 caps 미도착 상태에서 TerminalSlot 을 먼저 띄웠다가 caps 도착 후 RichSlot 으로 갈아끼면,
  //   스왑된 RichSlot 이 빈 채로 마운트돼 스왑 전 바이트가 영구 유실된다. 대신 caps(=AgentInfo) 도착 전엔
  //   중립 플레이스홀더만 두고(아래 연결 중·부재 막·대상 없음), 첫 구체 렌더러를 caps 확정 후 마운트해 assign
  //   시점 replay 를 온전히 받게 한다. (터미널 에이전트는 보통 assign 전에 AgentInfo 가 오므로 이 플레이스
  //   홀더는 일시적 엣지 상태다 — 터미널 replay 경로는 종전과 동일.)
  // 구조화 출력(NDJSON) = 라이브 RichSlot, 아니면 TerminalSlot(xterm) 분기 근거(ADR-0002/0044).
  const capsReady = slotAgentId != null && agent != null
  // ★기억은 같은 에이전트에만 유효하다★ — 배정이 바뀌면 무효(옛 모드로 새 에이전트를 그리면 안 된다).
  //   실제 폐기는 위 effect 가 한다(여기 판정만으로 남겨 두면 재배정 때 되살아난다).
  const kept =
    lastMountRef.current != null && lastMountRef.current.agentId === slotAgentId
      ? lastMountRef.current
      : null
  const renderAs = mode ?? kept?.mode
  const presence = slotAgentId != null ? agentPresence(slotAgentId, agents, profiles) : 'unknown'
  // ★ADR-0149 상태 판정 — 뷰를 지킬지의 축은 "기억 유무", 표시는 TRD S21-storage §6-8 의 표다★
  //   에이전트 있음                → 현행대로(caps 로 렌더러 결정)
  //   기억 있음 + 부재             → 뷰 유지(= keepDeadView). 흐림·심볼·입력차단은 슬롯 컴포넌트가 담당.
  //   명부·프로필 목록 미수신      → 「연결 중」 — ★이 경우에만★. 목록이 오면 반드시 아래 둘 중 하나로 넘어간다.
  //   프로필 있음 + 기억 없음      → 빈 슬롯 위에 부재 막(`SlotUnavailableVeil` — 흐림 + 심볼, 문구·단추 없음).
  //                                  끊기거나 죽은 슬롯과 같은 모습이고, 트리에서 활성화해 명부에 오르면 평소 화면으로
  //                                  바뀐다. ★ADR-0149 가 「삭제도 흐림으로 통일」을 거부한 사유 「빈 화면 흐림은
  //                                  오독된다」만, 이 기억 없음 경우에 한해 사용자 결정 2026-10-06 으로 뒤집었다★ —
  //                                  삭제(「대상 없음」)는 문구 그대로다(슬롯에서 활성화하는 UX 는 이 단계 밖).
  //                                  잎은 슬롯 컴포넌트를 띄우지 않을 때만 막을 그려 덮는 내용도, 슬롯별 판정의 중복도
  //                                  없다 — 막을 레이아웃 래퍼로 올리자는 안의 ADR-0149 거부는 그대로 선다. ADR-0149 (A) 의 「스폰
  //                                  대기도 연결 중」도 이것으로 대체된다 — 갓 활성화한 에이전트가 명부에 오르기 전 잠깐
  //                                  막이 보인다(수용).
  //   프로필 없음                  → 「대상 없음」. 배정은 그대로 두고 메뉴도 에이전트 슬롯 메뉴 그대로다(「비우기」·
  //                                  「에이전트 모니터링」으로 다른 내용을 놓는다 — 사용자 결정 2026-10-06).
  // ★"프로필이 없다"는 판정은 목록을 받은 뒤에만 신뢰한다★: refreshProfiles 는 재시도 없는 단발 pull 이라
  //   실패·지연이 실제로 가능하고, 그 구간에 presence 는 'unknown' 으로 보인다. 기억이 있는데 그걸로
  //   뷰를 내리면 보존하려던 대화가 그 자리에서 영구 소실된다(데몬 ring 도 이미 없다).
  const keepDeadView = agent == null && kept != null && (presence === 'reserved' || !profilesLoaded)
  // ADR-0149
  // ADR-0280
  const agentSlot: AgentSlotState | null =
    slotAgentId == null
      ? null
      : capsReady || keepDeadView
        ? 'view'
        : !agentsLoaded || !profilesLoaded
          ? 'connecting'
          : presence === 'reserved'
            ? 'stopped'
            : 'noTarget'
  // preset_palette·agent_list·usage variant 도 슬롯을 100% 채우는 실 렌더러라 hasContent=true(중앙정렬
  //   플레이스홀더 스타일이 이들 레이아웃을 깨지 않게, ADR-0060/0061/0062).
  const isPresetPalette = node.content.type === 'preset_palette'
  const isAgentList = node.content.type === 'agent_list'
  const isUsage = node.content.type === 'usage'
  const hasContent = agentSlot === 'view' || isPresetPalette || isAgentList || isUsage
  return (
    <div
      ref={borderRef}
      data-slot-border=""
      style={{
        position: 'absolute',
        inset: 0,
        background: 'var(--bg)',
        // border 폭을 항상 1px 고정해 포커스 이동 시 layout shift 제거.
        // 단축 속성(`border`)으로 합치지 않는다 — 폭이 `var()` 와 한 선언에 섞이면 jsdom 이 계산된 폭을 못 줘서
        //   위 지표 측정을 테스트로 잴 수 없다.
        borderWidth: 1,
        borderStyle: 'solid',
        borderColor: 'var(--border)',
        // ★포커스 링은 여기가 아니라 아래 absolute 오버레이로 그린다★: inset box-shadow 를 이 요소에 직접 주면
        //   overflow:hidden 슬롯에서 100% 채운 자식 컨텐츠(터미널 canvas·RichSlot)가 덮어 안 보였다(빈 슬롯만
        //   보이던 버그, 사용자 제보 2026-07-16). 이 요소가 absolute 라 오버레이의 앵커도 된다.
        boxSizing: 'border-box',
        // 콘텐츠(터미널/rich) 있을 때: 슬롯을 100% 채우도록 여백·정렬 제거(center 정렬 끼면 깨짐).
        // 빈 슬롯(empty): 플레이스홀더를 중앙정렬하는 flex 유지.
        ...(hasContent
          ? { overflow: 'hidden' }
          : {
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'var(--text-muted)',
              fontFamily: 'var(--font-ui)',
              fontSize: '12px',
              gap: '4px',
            }),
      }}
    >
      <SlotErrorBoundary
        slotId={node.id}
        resetKey={`${node.id}:${node.content.type}:${slotAgentId ?? ''}:${renderAs ?? ''}`}
      >
        {node.content.type === 'agent' ? (
          agentSlot === 'connecting' ? (
            <span>{t('agent.connecting')}</span>
          ) : agentSlot === 'stopped' ? (
            // 슬롯 컴포넌트를 띄우지 않는다 — 기억 없는(이 잎이 지금 그 에이전트의 마운트 기억을 쥐고 있지 않은 —
            //   마운트한 적이 없거나 재배정 때 기억을 버린) 슬롯이라 구독할 대상도 지킬 내용도 없다. 재시작·복원 뒤, 팝아웃·slot id 재마운트, A→B→A 재배정이 모두 여기로 온다.
            <SlotUnavailableVeil />
          ) : agentSlot === 'noTarget' ? (
            // 프로필도 없다(트리에서 삭제) — 배정 자체가 유효하지 않은 슬롯.
            // ★보존 중이던 대화가 여기서 사라지는 건 의도다★: 프로필 삭제는 사용자의 명시적 정리 동작이고,
            //   ADR-0149 가 그 경우의 표시를 문구로 정했다(뷰 유지 대상이 아니다).
            <span>{t('agent.noTarget')}</span>
          ) : (
            (() => {
              // ★viewId = node.id(slot id, ADR-0046)★: 슬롯이 자기 slot id 로 구독한다 — 같은 agentId 두
              //   슬롯도 독립 진도(버그 B 해소). key 도 slot id 로 두어(옛 agent_id 키는 같은 agent 두 슬롯이
              //   같은 React key 가 돼 remount 가 꼬였다) 슬롯 정체성을 slot 단위로 고정한다.
              // ★여기서 회차(epoch)를 내려보내지 않는다★: 슬롯의 재구독 트리거에서 화신을 뺐다. 이 자리가
              //   값을 만들어 내려보내면 그게 곧 재마운트 트리거라, 종료로 명부에서 수거되는 순간 값이
              //   떨어지며 **replay 가 오기도 전에** 보존하려던 대화가 지워진다(데몬 ring 은 이미 없다).
              //   화신 회전은 구독 쪽이 권위 명부로 판정해 비우기 신호로 낸다(protocolClient.observeRoster).
              switch (renderAs) {
                case 'dom':
                  // ★DOM 모드(§5 관측)★: 같은 출력 스트림을 평문 <pre> 로 그려 CDP eval/innerText 로 읽히게
                  // 한다(터미널 xterm 은 canvas 라 관측 불가).
                  return <DomSlot key={node.id} viewId={node.id} agentId={slotAgentId!} />
                case 'rich':
                  return <RichSlot key={node.id} viewId={node.id} agentId={slotAgentId!} />
                case 'terminal':
                default:
                  return <TerminalSlot key={node.id} viewId={node.id} agentId={slotAgentId!} />
              }
            })()
          )
        ) : node.content.type === 'preset_palette' ? (
          // 목록/추가/삭제는 PresetPalette 내부에서 agentClient(단일 제어 표면)로 흐른다.
          <PresetPalette />
        ) : node.content.type === 'agent_list' ? (
          // 조작은 AgentList 내부에서 agentClient/viewStore(단일 제어 표면)로 흐른다(§5).
          <AgentList />
        ) : node.content.type === 'usage' ? (
          // 값은 usageStore 미러에서만 읽는다 — 조회 수요는 셸이 이 슬롯 내용(show_*)으로 계산한다(TRD §1-7).
          <UsageSlot content={node.content} viewId={targetViewId} slotId={node.id} />
        ) : isForeign ? (
          // TRD S21-storage §6-2: 원문은 셸만 쥔다 — 메뉴는 빈 슬롯 메뉴 그대로이고, 거기서 내용을 놓으면 원문이 사라진다.
          <>
            <span>{t('slot.foreignContent')}</span>
            <span className="px-2 text-center wrap-anywhere" style={{ opacity: 0.7 }}>
              {t('slot.foreignContentHint')}
            </span>
          </>
        ) : (
          // ★순수 그림(ADR-0143)★: 표적은 슬롯 컨테이너다. 아이콘에 핸들러·tabIndex·role 을 되붙이면 컨테이너
          //   좌클릭과 겹쳐 메뉴가 두 번 열리고, 키보드로 못 빠져나오는 메뉴에 닿는 경로가 되살아난다.
          //   pointer-events 를 끊어 아이콘 위 클릭도 슬롯에 그대로 닿는다 — 유틸리티 클래스가 아니라 인라인인
          //   이유는 이 끊음이 스타일 취향이 아니라 동작 계약이라서다(클래스 규칙으로 덮이지 않고, Tailwind 를
          //   적용하지 않는 테스트 환경에서도 계산된 값으로 검증된다).
          <Plus className="size-11 text-muted" style={{ pointerEvents: 'none', opacity: 0.5 }} />
        )}
      </SlotErrorBoundary>
      {contextMenu && (
        // ADR-0064: 통합 슬롯 메뉴 — buildSlotMenu(content.type) 로 (콘텐츠 전용 ∪ 공통 '*') command 참조를
        //   결정적 정렬·resolve 해 항목을 만들고, ctx(viewId/slotId/agentId/content)를 넘긴다. command.run 은 그 넷으로
        //   백엔드 권위 경로(viewStore/agentClient)로 흐른다(§5 단일 제어 표면). content 종류가 가시성 게이트.
        <SlotMenu
          anchor={contextMenu}
          node={node}
          targetViewId={targetViewId}
          slotAgentId={slotAgentId}
          onClose={onCloseMenu}
        />
      )}
      {isFocused && (
        // ★강도 40%★: 옛 65%는 프레임이 시끄럽다고 판단해 하향(사용자 결정 2026-08-23 — 65→50→40을
        //   실제 화면에서 눈으로 비교해 고른 값). ADR-0066이 "너무 약해 안 보인다"며 거부했던 값과 같은
        //   40%지만, 그 거부는 실물 대조 없이 내린 것이라 이번 실측 결정이 우선한다. 되돌리려면 이 줄만
        //   올리면 된다. 세 테마 모두 color-mix 자동 적응. 제어 슬롯(트리/프리셋)은
        //   애초 focusSlot 제외(isContentSlot 게이트)라 isFocused=false → 링 없음(요구: 트리/프리셋 제외).
        // ADR-0280
        // ★링 색은 슬롯 상태에 따라 달라지면 안 된다(사용자 결정 2026-10-06)★ — 그래서 링은 부재 막(z-20, 잎이
        //   그리든 슬롯 컴포넌트가 그리든) 위에 서서 막이 링을 흐리게 덮지 않는다. 전제 둘: 이 요소 아래 막의 조상
        //   중 z-index 가 20 을 넘는 것이 없고, 링이 이 요소의 마지막 자식이다 — 그러면 z 가 같아 트리 순서상
        //   나중인 링이 위다(그 이상 올릴 필요 없다). 연결 안내(z 40)·메뉴(1000 이상) 아래다.
        // ★남은 차이(사용자 수용 2026-10-06 — 어색해 보이면 다시 연다)★: 링은 반투명(accent 40% 혼합)이라 보이는
        //   색은 여전히 바탕을 따른다 — 터미널 슬롯은 고정된 어두운 바탕이라 light·e-ink 테마에서 살아 있는 터미널의
        //   링과 막 덮인 슬롯의 링이 다르게 보인다.
        // pointer-events 를 끊어 클릭·메뉴는 슬롯에 그대로 닿는다.
        <div
          data-slot-focus-ring=""
          style={{
            position: 'absolute',
            inset: 0,
            pointerEvents: 'none',
            boxShadow: 'inset 0 0 0 1px color-mix(in srgb, var(--accent) 40%, transparent)',
            zIndex: 20,
          }}
        />
      )}
    </div>
  )
})

/**
 * 열린 슬롯 메뉴 — 메뉴 ctx 를 조립한다. ★메뉴가 보여 줄 스토어 상태가 생기면 여기서 구독해 ctx 에 싣는다(ADR-0252)★ —
 * 메뉴 기여는 ctx 만 읽고(ADR-0064), 여기서 구독하면 열린 메뉴도 그 상태를 따라 다시 그리고 구독은 메뉴가 열린
 * 동안만 산다. 지금 싣는 스토어 상태는 없다.
 */
function SlotMenu({
  anchor,
  node,
  targetViewId,
  slotAgentId,
  onClose,
}: {
  anchor: MenuAnchor
  node: SlotNode
  targetViewId: string | null
  slotAgentId: string | null
  onClose: () => void
}) {
  const content = node.content
  return (
    <SlotContextMenu
      x={anchor.x}
      y={anchor.y}
      items={buildSlotMenu(content.type)}
      ctx={{ viewId: targetViewId, slotId: node.id, agentId: slotAgentId, content }}
      onClose={onClose}
    />
  )
}

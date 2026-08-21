// ★유일한 레이아웃 렌더러★(Brick 1): 옛 프론트 전용 slotStore/LayoutRenderer(number id + content union)는
// 제거됐다. 이 렌더러는 wire LayoutNode(string UUID id + content: SlotContent, ADR-0060, src-tauri/bindings)만 그린다 —
// 사람 클릭(SlotContextMenu — 우클릭 전용, ADR-0144)이든 LLM(window.__engramLayout)이든 같은 invoke→emit
// 권위 루프로 갱신된다.

import { useEffect, useRef, useState } from 'react'
import { Allotment } from 'allotment'
import { Plus } from 'lucide-react'

import type { LayoutNode } from '../../api/layoutTypes'
import { useCurrentViewId, useViewStore } from '../../store/viewStore'
import { useAgentStore } from '../../store/agentStore'
import TerminalSlot from '../slot/TerminalSlot'
import RichSlot from '../slot/RichSlot'
import DomSlot from '../slot/DomSlot'
import PresetPalette from '../slot/PresetPalette'
import AgentList from '../agent/AgentList'
import { isContentSlot } from '../agent/selectOpenTarget'
import { agentPresence } from '../agent/mergeTreeNodes'
import SlotContextMenu from '../slot/SlotContextMenu'
import { buildSlotMenu } from '../../commands/slotMenu'
import { defaultRenderMode, type RenderMode } from '../slot/renderMode'
import { t } from '../../i18n'

export default function ViewLayoutRenderer({
  node,
  focusedSlotId,
  viewIdOverride,
}: {
  node: LayoutNode
  focusedSlotId: string | null
  // ★이 렌더러가 그리는 View id 오버라이드(선택).★ WindowLayout(main·팝업)이 각 탭 캔버스에 그 탭 view 를
  //   넘겨(ADR-0057) 내부 SlotContextMenu 의 액션 좌표를 그 탭 view 로 고정한다. 없으면 메뉴가
  //   useCurrentViewId(이 웹뷰 창의 active 탭) 폴백.
  viewIdOverride?: string | null
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
  //   없다. 렌더러 인스턴스는 슬롯 하나당 하나(재귀 렌더)라 이 ref 가 그 슬롯의 기억이다.
  //   ★데몬은 종료 시 replay ring 을 세션과 함께 버린다★ — 뷰를 내리면 그 대화는 재구독으로도 못 살린다.
  //   그래서 이 기억은 편의가 아니라 데이터 보존 수단이다.
  //   ★agentId 로 키잉하고 어긋나면 버린다★: 슬롯에 다른 에이전트를 배정하면 이 기억은 죽는다(아래 effect).
  //   무시만 하고 남겨 두면 그 에이전트를 다시 배정할 때 기억이 되살아나, 이미 언마운트된 빈 뷰를 흐림
  //   상태로 띄우고 수거된 에이전트로 구독까지 건다(ADR-0149 가 "빈 화면을 흐리게 하면 오독된다"고 거부한 상태).
  //   ★담는 것은 렌더 모드 하나뿐이다(ADR-0149 결정 5)★ — 회차 번호(epoch)는 여기도 슬롯 prop 에도 없다.
  //   슬롯의 재구독 트리거에서 화신을 뺐기 때문이다(비우기는 구독의 onReset 단독 — 각 슬롯 주석).
  //   ★알려진 한계 둘★ ① 대화를 보존 중인 슬롯을 **분할**하면 이 렌더러 인스턴스가 갈려 기억이 사라진다.
  //   기억을 밖(모듈 맵 등)으로 올려도 대화 자체는 슬롯 컴포넌트 state 에 있어 분할 시 언마운트로 함께
  //   사라지므로 — 빈 화면에 흐림만 남고 죽은 에이전트로 재구독까지 나간다 — 지금은 옛 동작(「연결 중」)으로
  //   떨어지게 둔다. 실제 해법은 대화 상태를 슬롯 컴포넌트 밖으로 올리는 것이고 그건 별건이다.
  //   ② 부재 구간에는 mode 유도가 없어 setRenderMode/clearRenderMode 가 조용히 무효다(호출은 성공으로 보인다).
  const lastMountRef = useRef<{ agentId: string; mode: RenderMode } | null>(null)
  // ★우클릭 슬롯 메뉴 상태(§5)★: 슬롯 하나당 이 렌더러 인스턴스가 하나라(재귀 렌더) 여기 useState 는
  //   그 슬롯 전용 메뉴 좌표다. 열림 시 SlotContextMenu 를 이 렌더러 안에서 직접 마운트한다 — 옛
  //   LayoutRenderer→SlotPane 래핑 경로가 Brick 1 에서 삭제돼 메뉴가 캔버스에서 닿지 않던 갭을 메운다.
  //   ★hooks 무조건 호출★: split/slot 분기 이전에 부른다(조건부 호출 금지).
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null)
  // ★이 메뉴가 조작할 View 좌표(ADR-0064)★: 옛 SlotContextMenu 내부 폴백을 여기(ctx 조립처)로 끌어올렸다.
  const currentViewId = useCurrentViewId()
  const targetViewId = viewIdOverride ?? currentViewId

  // ★slot 분기가 쓰는 값 셋을 hooks 위로 끌어올린다★: 아래 기억 effect 가 이 값들을 봐야 하고 hooks 는
  //   조건부로 부를 수 없다. split 노드에는 id·content 가 없으므로 전부 null 로 떨어진다.
  // ADR-0060: 슬롯 점유자 = SlotContent 태그드 유니온.
  const slotId = node.type === 'slot' ? node.id : null
  const slotAgentId =
    node.type === 'slot' && node.content.type === 'agent' ? node.content.agent_id : null
  const agent = slotAgentId != null ? (agents.find(a => a.id === slotAgentId) ?? null) : null
  const mode =
    agent != null && slotId != null ? (renderModeOverride[slotId] ?? defaultRenderMode(agent)) : null

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

  if (node.type === 'slot') {
    const isFocused = node.id === focusedSlotId
    // ★caps 도착 후에만 구체 렌더러를 마운트한다(ADR-0041 replay 소유권)★: 데몬 replay 는 slot-assign
    //   델타((window,agent) 키)에서 단 1회만 발화하고, 컴포넌트 스왑(TerminalSlot→RichSlot)엔 재발화하지
    //   않는다. 그래서 caps 미도착 상태에서 TerminalSlot 을 먼저 띄웠다가 caps 도착 후 RichSlot 으로 갈아끼면,
    //   스왑된 RichSlot 이 빈 채로 마운트돼 스왑 전 바이트가 영구 유실된다. 대신 caps(=AgentInfo) 도착 전엔
    //   중립 플레이스홀더만 두고(아래 '에이전트 연결 중…'), 첫 구체 렌더러를 caps 확정 후 마운트해 assign
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
    const presence = slotAgentId != null ? agentPresence(slotAgentId, agents, profiles) : 'unknown'
    // ★ADR-0148 상태 판정 — 축은 "기억 유무"다★
    //   에이전트 있음                → 현행대로(caps 로 렌더러 결정)
    //   프로필 있음 + 기억 없음      → 「연결 중」. 스폰 대기(예약 노드 활성화)·부팅 대기가 여기 든다 —
    //                                  아직 한 번도 뜬 적 없는 슬롯이라 보존할 것도, 죽었다고 알릴 것도 없다.
    //   프로필 있음 + 기억 있음      → 뷰 유지(= 이 분기). 흐림·심볼·입력차단은 슬롯 컴포넌트가 담당.
    //   프로필 없음                  → 「연결된 에이전트가 없습니다」(단 목록을 받은 뒤에만)
    // ★"프로필이 없다"는 판정은 목록을 받은 뒤에만 신뢰한다★: refreshProfiles 는 재시도 없는 단발 pull 이라
    //   실패·지연이 실제로 가능하고, 그 구간에 presence 는 'unknown' 으로 보인다. 기억이 있는데 그걸로
    //   뷰를 내리면 보존하려던 대화가 그 자리에서 영구 소실된다(데몬 ring 도 이미 없다).
    const keepDeadView = agent == null && kept != null && (presence === 'reserved' || !profilesLoaded)
    // preset_palette·agent_list variant 도 슬롯을 100% 채우는 실 렌더러라 hasContent=true(중앙정렬
    //   플레이스홀더 스타일이 이들 레이아웃을 깨지 않게, ADR-0060/0061/0062).
    const isPresetPalette = node.content.type === 'preset_palette'
    const isAgentList = node.content.type === 'agent_list'
    const hasContent = capsReady || keepDeadView || isPresetPalette || isAgentList
    return (
      <div
        style={{
          height: '100%',
          background: 'var(--bg)',
          // border 폭을 항상 1px 고정해 포커스 이동 시 layout shift 제거.
          border: '1px solid var(--border)',
          // ★포커스 링은 여기(래퍼)가 아니라 아래 absolute 오버레이로 그린다★: inset box-shadow 를 래퍼에
          //   직접 주면 overflow:hidden 슬롯에서 100% 채운 자식 컨텐츠(터미널 canvas·RichSlot)가 덮어 안
          //   보였다(빈 슬롯만 보이던 버그, 사용자 제보 2026-07-16). position:relative 로 오버레이 앵커만 잡는다.
          position: 'relative',
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
        // 슬롯 식별용 data 속성 — cdp eval 에서 DOM 으로 split 결과(슬롯 수)를 셀 수 있게.
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
        // ADR-0035: 메뉴 액션(분할/닫기/배정)은 viewStore(=window.__engramLayout) 단일 제어 표면으로만
        //   흐른다(사람 클릭 = LLM 이 한 표면, §5).
        onContextMenu={e => {
          e.preventDefault()
          setContextMenu({ x: e.clientX, y: e.clientY })
        }}
      >
        {node.content.type === 'agent' ? (
          agent == null && !keepDeadView ? (
            presence === 'reserved' || !agentsLoaded || !profilesLoaded ? (
              // 프로필은 있는데 아직 뜬 적 없다(스폰 대기·부팅 대기) 또는 목록 자체를 아직 못 받았다 —
              //   둘 다 "곧 온다" 라서 같은 문구다. ★목록 미수신을 「없습니다」로 새게 두면 안 된다★:
              //   refreshProfiles 는 재시도 없는 단발 pull 이라 실패·지연이 실제로 가능하고, 그때 대화를
              //   보존 중인 뷰가 조기 언마운트된다(profilesLoaded 가 그 방어선).
              <span>{t('agent.connecting')}</span>
            ) : (
              // 프로필도 없다(트리에서 삭제) — 배정 자체가 유효하지 않은 슬롯.
              // ★보존 중이던 대화가 여기서 사라지는 건 의도다★: 프로필 삭제는 사용자의 명시적 정리 동작이고,
              //   ADR-0149 가 그 경우의 표시를 문구로 정했다(뷰 유지 대상이 아니다).
              <span>{t('agent.noneConnected')}</span>
            )
          ) : (
            (() => {
              // ★viewId = node.id(slot id, ADR-0046)★: 슬롯이 자기 slot id 로 구독한다 — 같은 agentId 두
              //   슬롯도 독립 진도(버그 B 해소). key 도 slot id 로 두어(옛 agent_id 키는 같은 agent 두 슬롯이
              //   같은 React key 가 돼 remount 가 꼬였다) 슬롯 정체성을 slot 단위로 고정한다.
              // ★여기서 회차(epoch)를 내려보내지 않는다★: 슬롯의 재구독 트리거에서 화신을 뺐다. 이 자리가
              //   값을 만들어 내려보내면 그게 곧 재마운트 트리거라, 종료로 명부에서 수거되는 순간 값이
              //   떨어지며 **replay 가 오기도 전에** 보존하려던 대화가 지워진다(데몬 ring 은 이미 없다).
              //   화신 회전은 구독 쪽이 권위 명부로 판정해 비우기 신호로 낸다(protocolClient.observeRoster).
              const renderAs = mode ?? kept?.mode
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
        ) : (
          // ★순수 그림(ADR-0143)★: 표적은 슬롯 컨테이너다. 아이콘에 핸들러·tabIndex·role 을 되붙이면 컨테이너
          //   좌클릭과 겹쳐 메뉴가 두 번 열리고, 키보드로 못 빠져나오는 메뉴에 닿는 경로가 되살아난다.
          //   pointer-events 를 끊어 아이콘 위 클릭도 슬롯에 그대로 닿는다 — 유틸리티 클래스가 아니라 인라인인
          //   이유는 이 끊음이 스타일 취향이 아니라 동작 계약이라서다(클래스 규칙으로 덮이지 않고, Tailwind 를
          //   적용하지 않는 테스트 환경에서도 계산된 값으로 검증된다).
          <Plus className="size-11 text-muted" style={{ pointerEvents: 'none' }} />
        )}
        {contextMenu && (
          // ADR-0064: 통합 슬롯 메뉴 — buildSlotMenu(content.type) 로 (콘텐츠 전용 ∪ 공통 '*') command 참조를
          //   결정적 정렬·resolve 해 항목을 만들고, ctx(viewId/slotId/agentId)를 넘겨 각 command.run 이 백엔드
          //   권위 경로(viewStore/agentClient)로 흐르게 한다(§5 단일 제어 표면). content 종류가 가시성 게이트.
          <SlotContextMenu
            x={contextMenu.x}
            y={contextMenu.y}
            items={buildSlotMenu(node.content.type)}
            ctx={{ viewId: targetViewId, slotId: node.id, agentId: slotAgentId }}
            onClose={() => setContextMenu(null)}
          />
        )}
        {isFocused && (
          // ★강도 65%★: 너무 약한 포커스 표시가 반복 UX 불만이라(VS Code #24586 등, /research) "은은하되
          //   확실히 식별"되게 65%(사용자 결정). 세 테마 모두 color-mix 자동 적응. 제어 슬롯(트리/프리셋)은
          //   애초 focusSlot 제외(isContentSlot 게이트)라 isFocused=false → 링 없음(요구: 트리/프리셋 제외).
          <div
            style={{
              position: 'absolute',
              inset: 0,
              pointerEvents: 'none',
              boxShadow: 'inset 0 0 0 1px color-mix(in srgb, var(--accent) 65%, transparent)',
              zIndex: 10,
            }}
          />
        )}
      </div>
    )
  }
  // ★ADR-0140 유일한 진실 경계★: dir='top_bottom' = 위/아래 → allotment 의 vertical(수직 스택). 여기가
  //   뒤집히면 메뉴·타입·테스트가 전부 맞는데도 화면만 반대가 된다.
  // ★ratio 초기 사이징(ADR-0063)★: node.ratio = a(왼/위) 자식의 비율. ★Allotment 의 `defaultSizes` 는
  //   비율이 아니라 *픽셀*이다★ — [0.2,0.8] 을 주면 0.2px/0.8px 로 먹어 split-view 가 ~1px 로 붕괴하고
  //   자식들이 흐름 밖으로 쌓인다(실측 스샷으로 확인한 회귀). 대신 첫 pane(a=왼/위)에 `preferredSize` 를
  //   *퍼센트 문자열*로 줘 컨테이너 대비 비율로 배치한다(b 는 나머지 채움). 0.2 → "20%" = 20/80,
  //   0.5 → "50%" = 50/50. 컨테이너 실측 픽셀을 몰라도 되고 높이는 Allotment 가 컨테이너로 채운다.
  //   ★초기 사이징만★: 드래그 리사이즈→백엔드 ratio 되쓰기는 이 슬라이스 범위 밖(ADR-0063).
  // ★Allotment.Pane key = 위치 고정(pane-a/pane-b), 콘텐츠 파생 금지★: 옛 key 는 nodeKey(node.a) 로
  //   *서브트리 구조*에서 파생됐다 — 어느 pane 안의 슬롯이 split 으로 재구조화되면 그 pane 의 nodeKey 가
  //   바뀌어 React 가 Pane 을 unmount+remount 했고, Allotment 는 pane 이탈+합류로 보아 전 pane 을 균등
  //   재분배(형제의 비율 소실 — 예: 왼 20% → 50% 점프)했다. split 은 항상 a/b 두 자식을 이 순서로만
  //   가지므로 위치 기반 안정 key("pane-a"/"pane-b")를 쓴다 → 콘텐츠 재구조화에도 Pane 이 마운트 유지 →
  //   Allotment 가 사이즈를 보존한다. (형제 2개 사이에서만 유일하면 됨 — 중첩 Allotment 는 각자 짝을 가짐.)
  //   preferredSize(=ratio 파생 초기 사이징 %)는 첫 pane(a)에만 — 마운트 시 1회 적용·이후 보존(ADR-0063).
  return (
    <div style={{ height: '100%' }}>
      <Allotment vertical={node.dir === 'top_bottom'}>
        <Allotment.Pane key="pane-a" preferredSize={`${Math.round(node.ratio * 100)}%`}>
          <ViewLayoutRenderer node={node.a} focusedSlotId={focusedSlotId} viewIdOverride={viewIdOverride} />
        </Allotment.Pane>
        <Allotment.Pane key="pane-b">
          <ViewLayoutRenderer node={node.b} focusedSlotId={focusedSlotId} viewIdOverride={viewIdOverride} />
        </Allotment.Pane>
      </Allotment>
    </div>
  )
}
